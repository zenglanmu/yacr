//! Non-proxy `Extended` entity conversion (RTEXT, arc-aligned text, geo
//! position marker, section object, point clouds) plus the ATTRIB/ATTDEF text
//! helper.
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    /// Convert a non-proxy `Extended` entity by its class-specific payload.
    ///
    /// The acadrust `ExtendedEntityData` variants carry real structured fields;
    /// each is mapped to the semantic geometry this build can draw, with an
    /// honest [`Completeness`] for what is not applied. Variants with no display
    /// path at all stay [`SemanticGeometry::Opaque`]/[`Completeness::Unverified`].
    pub(crate) fn extended_semantics(
        &self,
        x: &acadrust::entities::ExtendedEntity,
    ) -> (SemanticGeometry, Completeness) {
        use acadrust::entities::ExtendedEntityData;
        match &x.data {
            ExtendedEntityData::RemoteText(d) => {
                let geometry = SemanticGeometry::Text {
                    text: d.text.clone(),
                    position: p3(d.position),
                    style: self.style_id(&d.style_name),
                    height: if d.height > 0.0 { d.height } else { 2.5 },
                    rotation: d.rotation,
                    font: self.style_font(&d.style_name),
                    h_align: TextAlignH::Left,
                    v_align: TextAlignV::Baseline,
                    // RTEXT carries no width boundary.
                    wrap_width: None,
                };
                let unhandled = rtext_unhandled_flags(d.flags);
                let normal = p3(d.normal);
                let off_plane = !cad_geometry::is_finite(normal)
                    || normal.x.abs() > 1e-6
                    || normal.y.abs() > 1e-6
                    || (normal.z.abs() - 1.0).abs() > 1e-6;
                let mut reasons: Vec<String> = Vec::new();
                if !unhandled.is_empty() {
                    reasons.push(format!(
                        "remote text generation flags are not applied: {}",
                        unhandled.join(", ")
                    ));
                }
                if off_plane {
                    reasons.push(
                        "remote text OCS normal is not applied; text is placed in world XY".into(),
                    );
                }
                if reasons.is_empty() {
                    (geometry, Completeness::Complete)
                } else {
                    (geometry, Completeness::Partial(reasons))
                }
            }
            ExtendedEntityData::ArcAlignedText(d) => (
                SemanticGeometry::Text {
                    text: d.text.clone(),
                    position: p3(d.center),
                    style: self.style_id(&d.style_name),
                    height: if d.text_size > 0.0 { d.text_size } else { 2.5 },
                    rotation: 0.0,
                    font: self.style_font(&d.style_name),
                    h_align: TextAlignH::Left,
                    v_align: TextAlignV::Baseline,
                    // Arc-aligned text carries no width boundary.
                    wrap_width: None,
                },
                Completeness::Partial(vec![
                    "arc-aligned placement approximated as linear text".into()
                ]),
            ),
            ExtendedEntityData::GeoPositionMarker(d) => {
                let mut children: Vec<SemanticGeometry> = Vec::new();
                if d.radius.is_finite() && d.radius > 0.0 {
                    children.push(SemanticGeometry::Circle {
                        center: p3(d.position),
                        normal: Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 1.0,
                        },
                        radius: d.radius,
                    });
                }
                if !d.notes.is_empty() {
                    let height = if d.radius.is_finite() && d.radius > 0.0 {
                        d.radius * 0.5
                    } else {
                        2.5
                    };
                    children.push(SemanticGeometry::Text {
                        text: d.notes.clone(),
                        position: p3(d.position),
                        style: StyleId(0),
                        height,
                        rotation: 0.0,
                        font: None,
                        h_align: TextAlignH::Center,
                        v_align: TextAlignV::Middle,
                        wrap_width: None,
                    });
                }
                if d.mtext_visible {
                    if let Some(mtext) = &d.embedded_mtext {
                        let (h_align, v_align) = attach_align(mtext.attachment_point);
                        children.push(SemanticGeometry::Text {
                            text: mtext.value.clone(),
                            position: p3(mtext.insertion_point),
                            style: self.style_id(&mtext.style),
                            height: mtext.height,
                            rotation: mtext.rotation,
                            font: self.style_font(&mtext.style),
                            h_align,
                            v_align,
                            // Embedded MTEXT keeps its width boundary dropped in
                            // this build; only MLEADER populates it.
                            wrap_width: None,
                        });
                    }
                }
                let geometry = if children.is_empty() {
                    SemanticGeometry::Opaque {
                        type_key: "POSITIONMARKER".into(),
                        version: 1,
                        payload: Vec::new(),
                    }
                } else {
                    SemanticGeometry::Compound(children)
                };
                (
                    geometry,
                    Completeness::Partial(vec![
                        "geo position marker drawn as a radius circle plus notes text; \
                         text_alignment, enable_frame_text and landing_gap are not applied"
                            .into(),
                    ]),
                )
            }
            ExtendedEntityData::SectionObject(d) => {
                let mut children: Vec<SemanticGeometry> = Vec::new();
                if d.vertices.len() >= 2 {
                    children.push(polyline_semantics(
                        d.vertices.iter().map(|v| p3(*v)).collect(),
                        d.vertices.len() >= 3,
                    ));
                }
                if d.back_line_vertices.len() >= 2 {
                    children.push(polyline_semantics(
                        d.back_line_vertices.iter().map(|v| p3(*v)).collect(),
                        d.back_line_vertices.len() >= 3,
                    ));
                }
                let geometry = if children.is_empty() {
                    SemanticGeometry::Opaque {
                        type_key: "SECTIONOBJECT".into(),
                        version: 1,
                        payload: Vec::new(),
                    }
                } else {
                    SemanticGeometry::Compound(children)
                };
                (
                    geometry,
                    Completeness::Partial(vec![
                        "section object outline drawn; top/bottom height extrusion, state/indicator \
                         styling and the external settings handle are not applied"
                            .into(),
                    ]),
                )
            }
            ExtendedEntityData::PointCloud(d) => {
                point_cloud_box("ACDBPOINTCLOUD", d.extents_min, d.extents_max)
            }
            ExtendedEntityData::PointCloudEx(d) => {
                point_cloud_box("ACDBPOINTCLOUDEX", d.extents_min, d.extents_max)
            }
            // New in acadrust 0.6.8: ACAD Architecture (AEC) entities carry the
            // profiles decoded from their proxy-graphics preview. Those profiles
            // are world coordinates, so they can be drawn directly; the class
            // parameters (openings, 3D massing, style objects) stay unmodeled.
            ExtendedEntityData::Arch(d) => arch_semantics(x.class_name(), d),
            // No display path by design, or content that is not loaded from a
            // drawing file: kept explicit Opaque/Unverified rather than faked.
            ExtendedEntityData::Camera { .. }
            | ExtendedEntityData::DynamicBlock(_)
            | ExtendedEntityData::CoordinationModel(_)
            | ExtendedEntityData::OleFrame(_)
            | ExtendedEntityData::LayoutPrintConfig(_)
            | ExtendedEntityData::Format(_)
            | ExtendedEntityData::Legacy(_)
            | ExtendedEntityData::RegisteredClass(_)
            | ExtendedEntityData::Proxy(_) => (
                SemanticGeometry::Opaque {
                    type_key: x.class_name().to_string(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Unverified,
            ),
        }
    }

    /// Text geometry for an ATTRIB/ATTDEF value, at its insertion point and
    /// using its own text style.
    pub(crate) fn attribute_text(
        &self,
        style: &str,
        text: String,
        position: Point3,
        height: f64,
        rotation: f64,
    ) -> SemanticGeometry {
        SemanticGeometry::Text {
            text,
            position,
            style: self.style_id(style),
            height,
            rotation,
            font: self.style_font(style),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            // ATTRIB/ATTDEF text has no width boundary.
            wrap_width: None,
        }
    }
}

/// The RTEXT generation-flag bits this build cannot apply, by name.
///
/// Bits `0x2` (backward) and `0x4` (upside-down) are the known TEXT generation
/// flags; any other set bit is reported as an unknown mask. An empty result
/// means every bit was interpreted.
fn rtext_unhandled_flags(flags: i16) -> Vec<String> {
    let mut out = Vec::new();
    if flags & 0x2 != 0 {
        out.push("backward".to_string());
    }
    if flags & 0x4 != 0 {
        out.push("upside-down".to_string());
    }
    let rest = flags & !(0x2 | 0x4);
    if rest != 0 {
        out.push(format!("unknown bits 0x{:X}", rest as u16));
    }
    out
}

/// ACAD Architecture (AEC) entity geometry.
///
/// A straight AEC wall is drawn from its **native** parameters when acadrust can
/// decode them (`ArchEntityData::straight_wall_geometry`, DWG 2027 layout only):
/// the oriented footprint from [`wall_footprint`] replaces the preview profile.
///
/// Otherwise acadrust 0.6.8 decodes the entity's proxy-graphics preview into
/// profiles in **world coordinates** (`ArchEntityData::profiles`), and each
/// profile with at least two points is drawn as an open polyline. An open
/// polyline whose last point repeats the first renders exactly like the closed
/// profile, so closure needs no invented flag: the encoded record carries none.
///
/// Everything else the class body holds — openings cut out of walls, door and
/// window bodies, 3D massing, style/anchor objects — is **not** modeled here and
/// is reported by name instead of being faked.
fn arch_semantics(
    class_name: &str,
    data: &acadrust::classes::arch::ArchEntityData,
) -> (SemanticGeometry, Completeness) {
    // Native straight-wall parameters are authored geometry, unlike the
    // proxy-graphics preview profile, so they win when present.
    if let Some(geometry) = data.straight_wall_geometry() {
        if let Some(outline) = wall_footprint(&geometry) {
            return (
                SemanticGeometry::Compound(vec![polyline_semantics(outline, true)]),
                Completeness::Partial(vec![
                    "wall footprint is computed from the native start/end and thickness; height \
                     is not drawn, openings are not cut, and shared-endpoint miters are not \
                     applied (each wall keeps square ends)"
                        .into(),
                ]),
            );
        }
    }
    let mut children: Vec<SemanticGeometry> = Vec::new();
    for profile in &data.profiles {
        if profile.len() < 2 {
            continue;
        }
        children.push(polyline_semantics(
            profile.iter().map(|point| p3(*point)).collect(),
            false,
        ));
    }
    if children.is_empty() {
        return (
            SemanticGeometry::Opaque {
                type_key: class_name.to_string(),
                version: 1,
                payload: Vec::new(),
            },
            Completeness::Partial(vec![
                "AEC arch entity has no decodable proxy-graphics profile; only its class name is \
                 preserved and no geometry is drawn"
                    .into(),
            ]),
        );
    }
    (
        SemanticGeometry::Compound(children),
        Completeness::Partial(vec![
            "AEC arch geometry is the proxy-graphics preview profile only; openings, 3D massing, \
             class parameters and style objects are not modeled"
                .into(),
        ]),
    )
}

/// Oriented footprint of a straight AEC wall from its native parameters.
///
/// The outline is the squared ribbon around the stored baseline
/// (`geometry.start` → `geometry.end`), offset half the full `geometry.width`
/// to each side, with the first point repeated at the end (so the caller must
/// treat it as closed). This is the authored footprint; the stored
/// proxy-graphics preview profile is not used when this exists. Returns `None`
/// when acadrust cannot build a finite outline (degenerate baseline, non-positive
/// thickness, non-finite coordinates).
pub(crate) fn wall_footprint(
    geometry: &acadrust::classes::arch::StraightWallGeometry,
) -> Option<Vec<Point3>> {
    let outline = acadrust::classes::arch::continuous_wall_outline(
        &[geometry.start, geometry.end],
        geometry.width,
    )?;
    if outline.len() < 4 {
        return None;
    }
    let points: Vec<Point3> = outline.iter().map(|point| p3(*point)).collect();
    points
        .iter()
        .all(|point| cad_geometry::is_finite(*point))
        .then_some(points)
}

/// Test access to [`wall_footprint`].
///
/// This module is private to `entity` and the glob re-export there is not
/// `pub(crate)`, so the shared display-closure test module (declared at the crate
/// root) cannot name the free function. Inherent methods are reachable
/// through the crate-visible type, so this `cfg(test)` shim keeps the contract
/// tests in `tests_display_table_extended.rs` without widening the module graph.
#[cfg(test)]
impl<'a> ImporterBuilder<'a> {
    pub(crate) fn wall_footprint_for_test(
        geometry: &acadrust::classes::arch::StraightWallGeometry,
    ) -> Option<Vec<Point3>> {
        wall_footprint(geometry)
    }
}

/// A point-cloud extents box as its twelve edges.
///
/// The scan points themselves are not carried by acadrust 0.6.8, so only the
/// stored extents are drawn; the result is honestly `Partial`.
fn point_cloud_box(
    class_name: &str,
    extents_min: acadrust::Vector3,
    extents_max: acadrust::Vector3,
) -> (SemanticGeometry, Completeness) {
    let lo = Point3 {
        x: extents_min.x.min(extents_max.x),
        y: extents_min.y.min(extents_max.y),
        z: extents_min.z.min(extents_max.z),
    };
    let hi = Point3 {
        x: extents_min.x.max(extents_max.x),
        y: extents_min.y.max(extents_max.y),
        z: extents_min.z.max(extents_max.z),
    };
    let corner = |index: usize| Point3 {
        x: if index & 1 == 0 { lo.x } else { hi.x },
        y: if index & 2 == 0 { lo.y } else { hi.y },
        z: if index & 4 == 0 { lo.z } else { hi.z },
    };
    let mut children: Vec<SemanticGeometry> = Vec::new();
    for index in 0..8usize {
        for bit in [1usize, 2, 4] {
            if index & bit == 0 {
                push_edge(&mut children, corner(index), corner(index | bit));
            }
        }
    }
    let (geometry, reason) = if children.is_empty() {
        (
            SemanticGeometry::Opaque {
                type_key: class_name.to_string(),
                version: 1,
                payload: Vec::new(),
            },
            "point data is not available from acadrust 0.6.8; the stored extents are degenerate, \
             so no box is drawn"
                .to_string(),
        )
    } else {
        (
            SemanticGeometry::Compound(children),
            "point data is not available from acadrust 0.6.8; extents box shown".to_string(),
        )
    };
    (geometry, Completeness::Partial(vec![reason]))
}
