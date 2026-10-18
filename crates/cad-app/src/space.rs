use super::*;

/// Validate that model space or a named paper layout can be drawn.
///
/// `SpaceSelection::Model` is always valid. A paper selection must name a layout
/// that exists in `database` **and** whose viewports this build can draw; an
/// unknown layout is `InvalidInput` and a known-but-undrawable one is
/// `Unsupported` with the representation layer's own reason. This is what keeps
/// an unsupported layout an explicit diagnostic instead of a blank success
/// (audit F04/B22).
pub fn validate_space(database: &DrawingDatabase, space: SpaceSelection) -> CadResult<()> {
    match space {
        SpaceSelection::Model => Ok(()),
        SpaceSelection::Paper(id) => match enumerate_layouts(database)
            .into_iter()
            .find(|descriptor| descriptor.id == id)
        {
            None => Err(CadError::InvalidInput(format!(
                "unknown paper layout {}",
                id.0
            ))),
            Some(descriptor) if !descriptor.supported => Err(CadError::Unsupported(format!(
                "layout {:?} cannot be drawn: {}",
                descriptor.name, descriptor.reason
            ))),
            Some(_) => Ok(()),
        },
    }
}

/// Validate the domain `SpaceId` payload form of a space switch.
///
/// `SpaceId::Block` is block-definition space, which is reached only through an
/// `INSERT` and is never a selectable observation space, so it is refused.
pub fn validate_space_id(database: &DrawingDatabase, space: &SpaceId) -> CadResult<()> {
    match space {
        SpaceId::Model => Ok(()),
        SpaceId::Paper(id) => validate_space(database, SpaceSelection::Paper(*id)),
        SpaceId::Block(_) => Err(CadError::InvalidInput(
            "block space is not a selectable drawing space".into(),
        )),
    }
}

/// The measurement space for a pick in the session's active space.
///
/// Model space keeps the existing split (planar algorithms use the viewport work
/// plane, spatial ones use [`MeasurementSpace::World3d`]). Paper space measures
/// the sheet directly, so a distance becomes a planar paper distance and an
/// angle (undefined on a 2D sheet) is refused rather than guessed. This is what
/// keeps a paper distance distinguishable from a model distance (F04/F06).
pub fn resolve_measurement_space(
    active_space: &SpaceId,
    work_plane: WorkPlane,
    algorithm: MeasurementAlgorithm,
) -> CadResult<(MeasurementAlgorithm, MeasurementSpace)> {
    match active_space {
        SpaceId::Model => {
            let space = match algorithm {
                MeasurementAlgorithm::Distance3d | MeasurementAlgorithm::Angle3Points => {
                    MeasurementSpace::World3d
                }
                _ => MeasurementSpace::Plane(work_plane),
            };
            Ok((algorithm, space))
        }
        SpaceId::Paper(layout) => match algorithm {
            // A 3D distance on a 2D sheet is a planar paper distance.
            MeasurementAlgorithm::Distance3d => Ok((
                MeasurementAlgorithm::Distance2d,
                MeasurementSpace::Paper(*layout),
            )),
            MeasurementAlgorithm::Angle3Points => Err(CadError::Unsupported(format!(
                "angle is not defined in paper space of layout {}; switch to model space",
                layout.0
            ))),
            _ => Ok((algorithm, MeasurementSpace::Paper(*layout))),
        },
        SpaceId::Block(_) => Err(CadError::InvalidInput(
            "block space is not a selectable drawing space".into(),
        )),
    }
}

/// The [`MeasurementSpace`] for measuring *inside* one layout viewport.
///
/// The returned space carries the viewport's verified paper→model inverse, so a
/// pick on the sheet measures real model distance. A viewport whose state cannot
/// be represented has no valid inverse and is refused with the representation
/// layer's stable reason: model measurement is disabled, never guessed from
/// paper pixels (F04).
pub fn viewport_measurement_space(
    database: &DrawingDatabase,
    layout_id: LayoutId,
    viewport_index: usize,
) -> CadResult<MeasurementSpace> {
    let layout = database
        .layout(layout_id)
        .ok_or_else(|| CadError::InvalidInput(format!("unknown paper layout {}", layout_id.0)))?;
    let viewport = layout.viewports.get(viewport_index).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "layout {} has no viewport {}",
            layout_id.0, viewport_index
        ))
    })?;
    match viewport_transform(viewport) {
        ViewportState::Supported(t) => Ok(MeasurementSpace::ViewportModel {
            layout: layout_id,
            inverse: t.to_model,
        }),
        ViewportState::Unsupported(why) => Err(CadError::Unsupported(format!(
            "layout {} viewport {} cannot be measured in model space: {why}",
            layout_id.0, viewport_index
        ))),
    }
}
