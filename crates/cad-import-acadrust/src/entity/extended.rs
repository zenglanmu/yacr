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

/// A point-cloud extents box as its twelve edges.
///
/// The scan points themselves are not carried by acadrust 0.6.3, so only the
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
            "point data is not available from acadrust 0.6.3; the stored extents are degenerate, \
             so no box is drawn"
                .to_string(),
        )
    } else {
        (
            SemanticGeometry::Compound(children),
            "point data is not available from acadrust 0.6.3; extents box shown".to_string(),
        )
    };
    (geometry, Completeness::Partial(vec![reason]))
}
