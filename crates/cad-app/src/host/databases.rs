use super::*;

/// Synthetic drawing used before a real DWG is opened.
///
/// This is **not** a compatibility claim (the fixture manifest holds only
/// synthetic fixtures and one openly-distributed sample); it exercises the
/// shared UI/render pipeline with known geometry.
pub fn demo_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layer(Layer {
            id: LayerId(1),
            name: "WALLS".into(),
            visible: true,
        })
        .unwrap();

    let mut push = |id: u128, geometry: SemanticGeometry, layer: LayerId, order: i64| {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: class_name(&geometry).into(),
                    revision: Revision(0),
                    source_handle: Some(format!("{id:X}")),
                },
                id: EntityId(id),
                layer,
                space: SpaceId::Model,
                geometry,
                draw_order: order,
            })
            .unwrap();
    };

    // A simple room outline with a circle and a polyline-with-bulge.
    push(
        1,
        SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(4000.0, 0.0),
        },
        LayerId(1),
        0,
    );
    push(
        2,
        SemanticGeometry::Line {
            start: p(4000.0, 0.0),
            end: p(4000.0, 3000.0),
        },
        LayerId(1),
        1,
    );
    push(
        3,
        SemanticGeometry::Line {
            start: p(4000.0, 3000.0),
            end: p(0.0, 3000.0),
        },
        LayerId(1),
        2,
    );
    push(
        4,
        SemanticGeometry::Line {
            start: p(0.0, 3000.0),
            end: p(0.0, 0.0),
        },
        LayerId(1),
        3,
    );
    push(
        5,
        SemanticGeometry::Circle {
            center: p(2000.0, 1500.0),
            normal: pz(0.0, 0.0, 1.0),
            radius: 800.0,
        },
        LayerId(0),
        4,
    );
    push(
        6,
        SemanticGeometry::Polyline {
            points: vec![p(500.0, 500.0), p(1500.0, 500.0), p(1500.0, 1000.0)],
            bulges: vec![0.0, 1.0, 0.0],
            closed: false,
        },
        LayerId(0),
        5,
    );
    builder.finish().unwrap()
}

/// Empty drawing with only the required layer `"0"`.
///
/// This is the document a host installs for "New": no entities, no bounds, and
/// exactly the default layer every DWG must contain. It makes no compatibility
/// or content claim.
pub fn blank_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.finish().unwrap()
}

pub(super) fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn pz(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn class_name(geometry: &SemanticGeometry) -> &'static str {
    crate::app_drawing::class_name(geometry)
}

/// Build the local snap targets for a drawing's model-space geometry.
///
/// [`crate::drawing_pick_items`] expands `INSERT`s and returns each item's
/// geometry in its **own local** coordinates plus the accumulated world
/// transform; the snap engine has no transform parameter, so the transform is
/// baked into the geometry with [`transform_geometry`]. An item whose kind this
/// build cannot transform exactly (text, spline, shape, opaque, compound) is
/// **skipped**, never snapped at a wrong location; a top-level entity carries
/// the identity transform and is passed through unchanged (so a model-space
/// spline still snaps to its sampled points).
///
/// Every expanded item is presented in **world model space** (block children
/// are inlined by their composed transform), so all targets are tagged
/// [`SpaceId::Model`]. The precision mirrors the source: a spline's snap points
/// come from a fixed sample set ([`Precision::Approximate`]); analytic kinds are
/// [`Precision::Analytic`].
fn snap_targets_for(drawing: &DrawingDatabase, document: DocumentId) -> Vec<SnapTarget> {
    let identity = Transform3::identity();
    crate::drawing_pick_items(drawing, document)
        .into_iter()
        .filter_map(|item| {
            let geometry = if item.transform == identity {
                item.geometry
            } else {
                match transform_geometry(&item.geometry, &item.transform) {
                    Ok(world) => world,
                    // A kind this build cannot bake exactly contributes nothing
                    // rather than a snap at an un-transformed location.
                    Err(_) => return None,
                }
            };
            let precision = match &geometry {
                SemanticGeometry::Spline { .. } => Precision::Approximate { error_bound: None },
                _ => Precision::Analytic,
            };
            Some(SnapTarget::new(
                geometry,
                item.source,
                SpaceId::Model,
                precision,
            ))
        })
        .collect()
}

/// Snap candidates near a world cursor on the model work plane.
///
/// This is the pure bridge between the database and the object-snap overlay:
/// the caller (a host) holds the live cursor and the viewport's
/// `world_per_px`, and gets back the real [`SnapCandidate`]s the measurement
/// snap engine resolves, ready to hand to `CadView::set_snap_hints`.
///
/// **Cursor/ray convention.** A measurement or draw cursor is a world point on
/// the planar work plane, not a screen ray. The pick ray is therefore built as
/// `origin = cursor`, `direction = +Z`, and the snap plane is the world-Z plane
/// through the cursor (`origin = cursor`, `u = +X`, `v = +Y`). With a plane the
/// engine uses the ray∩plane point as its proximity probe, so the probe is
/// exactly `cursor`; candidate points on that plane have ray parameter `t = 0`
/// and are accepted (`accept` only rejects `t < 0`). This mirrors the
/// engine's own test convention (`snap.rs` builds a downward ray onto `z = 0`)
/// but keeps the plane at the cursor's height so a non-zero-z planar drawing is
/// not silently shifted.
///
/// Only geometry exactly on the cursor's plane is in front of the ray; that is
/// the planar 2D contract, stated rather than faked. `space` is the session's
/// active space and is passed as the engine's space filter; because
/// [`crate::drawing_pick_items`] only walks **model** space, a non-model `space`
/// honestly yields no candidates (paper-space snapping is not built here).
///
/// Returns [`CadError::InvalidInput`] for a non-finite cursor or a
/// non-positive/non-finite `world_per_px`; it never fabricates a candidate.
pub fn snap_candidates_near(
    drawing: &DrawingDatabase,
    document: DocumentId,
    cursor: Point3,
    world_per_px: f64,
    space: SpaceId,
) -> CadResult<Vec<SnapCandidate>> {
    if !cursor.x.is_finite() || !cursor.y.is_finite() || !cursor.z.is_finite() {
        return Err(CadError::InvalidInput(
            "snap cursor must be finite".to_string(),
        ));
    }
    if !world_per_px.is_finite() || world_per_px <= 0.0 {
        return Err(CadError::InvalidInput(
            "snap world_per_px must be finite and positive".to_string(),
        ));
    }
    let plane = WorkPlane {
        origin: cursor,
        u: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        v: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    };
    let ray = Ray3 {
        origin: cursor,
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    };
    let targets = snap_targets_for(drawing, document);
    MeasurementEngine::default().snap_targets(
        &targets,
        &ray,
        Some(&plane),
        world_per_px,
        Some(space),
    )
}
