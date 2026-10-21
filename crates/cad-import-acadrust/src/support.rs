//! Support classification, proxy policy and document identity.

use super::*;

/// Completeness reason (a) for a tessellated ACIS entity.
///
/// The tolerance is embedded so the report says **which** discretisation was
/// used, not merely that one was used.
pub(crate) fn acis_approximation_reason(linear_deflection: f64) -> String {
    format!(
        "ACIS solid drawn as a discrete mesh approximation at tolerance \
         {linear_deflection:.6e}; it does not refine when you zoom"
    )
}

/// Completeness reason (b) for a tessellated ACIS entity: the source is gone.
pub(crate) const ACIS_SOURCE_NOT_RETAINED_REASON: &str = "the ACIS source is not \
retained: the entity carries tessellated mesh geometry instead of its original \
SAT/SAB bytes, so it cannot be round-tripped as a 3DSOLID";

impl<'a> ImporterBuilder<'a> {
    /// Lift + tessellate one ACIS entity at import time.
    ///
    /// The importer is the only crate above the kernel seam that may parse
    /// SAT/SAB, so this is where the four ACIS entity types become displayable
    /// [`SemanticGeometry::Mesh`]. Success is never reported `Complete`: the
    /// facets are a discrete approximation and the ACIS source is dropped.
    /// A payload the kernel refuses stays [`SemanticGeometry::Opaque`] with the
    /// kernel's own named reason instead of an empty success.
    pub(crate) fn acis_semantics(
        &mut self,
        entity: &EntityType,
        acis: &acadrust::entities::AcisData,
    ) -> (SemanticGeometry, Completeness) {
        use cad_kernel_adapter::{
            tessellate_exchange, SolidExchange, TessellationBudget, TessellationTolerance,
        };

        let exchange = acis_exchange(acis);
        let (tolerance, _diagonal) = match &exchange {
            SolidExchange::Brep(brep) => crate::solid::acis_tolerance(brep),
            // Raw SAT/SAB that acadrust could not parse is refused below; the
            // tolerance is irrelevant to that refusal.
            _ => (TessellationTolerance::default(), 0.0),
        };
        let outcome = tessellate_exchange(&exchange, tolerance, TessellationBudget::default());

        if let Some(mesh) = outcome.mesh() {
            if !mesh.is_empty() {
                let mut reasons = vec![
                    acis_approximation_reason(tolerance.linear_deflection),
                    ACIS_SOURCE_NOT_RETAINED_REASON.to_string(),
                ];
                // A partially tessellated body keeps the kernel's degradation
                // reasons (missing faces / open edges), never hides them.
                reasons.extend(
                    outcome
                        .diagnostics()
                        .iter()
                        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message)),
                );
                let completeness = self.acis_partial("import.acis_tessellated", reasons);
                return (SemanticGeometry::Mesh(mesh.mesh.clone()), completeness);
            }
        }

        let reasons = acis_refusal_reasons(&outcome);
        let completeness = self.acis_partial("import.acis_unsupported", reasons);
        let (version, payload) = acis_raw_payload(acis);
        (
            SemanticGeometry::Opaque {
                type_key: entity_class_name(entity),
                version,
                payload,
            },
            completeness,
        )
    }

    /// Record each named reason once per import and return the entity verdict.
    fn acis_partial(&mut self, code: &str, reasons: Vec<String>) -> Completeness {
        for reason in &reasons {
            if self.reported_reasons.insert(reason.clone()) {
                self.diagnostics.push(Diagnostic {
                    object: None,
                    code: code.into(),
                    message: reason.clone(),
                });
            }
        }
        Completeness::Partial(reasons)
    }
}

/// The kernel's own named reason(s) for refusing an ACIS payload.
fn acis_refusal_reasons(outcome: &cad_kernel_adapter::TessellationOutcome) -> Vec<String> {
    use cad_kernel_adapter::TessellationOutcome;
    let mut reasons: Vec<String> = match outcome {
        TessellationOutcome::Unsupported { reason } => {
            vec![format!("{}: {}", reason.code, reason.detail)]
        }
        TessellationOutcome::Failed { diagnostics } => diagnostics
            .iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect(),
        // `Success`/`Partial` with no mesh is impossible today, but must not be
        // reported as a silent empty success if it ever happens.
        _ => Vec::new(),
    };
    if reasons.is_empty() {
        reasons.push("kernel produced no tessellatable geometry".to_string());
    }
    reasons
}

/// Whether the display pipeline can actually draw this geometry (audit B20).
///
/// Text has no scene batching yet and opaque geometry has no representation, so
/// neither is `Verified`; an INSERT depends on its block contents and is
/// resolved by the caller. A tessellated ACIS solid is a real [`Mesh`] and is
/// therefore drawable (`Verified`); only a refused ACIS payload stays `Opaque`.
pub(crate) fn display_support(geometry: &SemanticGeometry) -> (SupportStatus, SupportStatus) {
    match geometry {
        SemanticGeometry::Opaque { .. } => (SupportStatus::Unsupported, SupportStatus::Unsupported),
        // Tessellated ACIS solids (3DSOLID/BODY/REGION/SURFACE) arrive as Mesh:
        // they now have a display, so they must not inherit the Opaque refusal.
        SemanticGeometry::Mesh(_) => (SupportStatus::Verified, SupportStatus::Verified),
        // Text is a supported entity kind whose display is conditional on a
        // host font: the host registers arbitrary `name=path` keys and legacy
        // DXF names such as `txt` omit the `.shx` extension, so availability is
        // a host-resource question, not a missing representation. Whether the
        // font resolves is reported by the representation/text build, not here.
        SemanticGeometry::Text { .. } => (SupportStatus::Unverified, SupportStatus::Unverified),
        SemanticGeometry::Insert { .. } => (SupportStatus::Unverified, SupportStatus::Unverified),
        SemanticGeometry::Compound(children) => {
            let mut render = SupportStatus::Verified;
            let mut pick = SupportStatus::Verified;
            for child in children {
                let (r, p) = display_support(child);
                render = weaker(render, r);
                pick = weaker(pick, p);
            }
            (render, pick)
        }
        // A dimension inherits the weakest status of its display children, so a
        // dimension with a `Text` child is `Unverified` (not the false
        // `Verified` the wildcard arm would give it; ADR 0017 A.8).
        SemanticGeometry::Dimension(dimension) => {
            let mut render = SupportStatus::Verified;
            let mut pick = SupportStatus::Verified;
            for child in dimension.display() {
                let (r, p) = display_support(child);
                render = weaker(render, r);
                pick = weaker(pick, p);
            }
            (render, pick)
        }
        // A hatch mirrors the compound recursion, and additionally refuses to
        // claim `Verified` when its display lost the fill/pattern (boundary-only
        // because the fill failed, the gradient kind is unsupported or a pattern
        // had no families). The wildcard arm used to report every hatch as
        // `Verified` (ADR 0018 E).
        SemanticGeometry::Hatch(hatch) => {
            let mut render = SupportStatus::Verified;
            let mut pick = SupportStatus::Verified;
            for child in hatch.display() {
                let (r, p) = display_support(child);
                render = weaker(render, r);
                pick = weaker(pick, p);
            }
            if !hatch_display_is_complete(hatch) {
                render = weaker(render, SupportStatus::Partial);
                pick = weaker(pick, SupportStatus::Partial);
            }
            (render, pick)
        }
        // A multileader inherits the weakest status of its display children, so
        // one with a `Text` child is `Unverified` rather than the false
        // `Verified` the wildcard arm would give it (ADR 0019 decision G).
        SemanticGeometry::MultiLeader(multileader) => {
            let mut render = SupportStatus::Verified;
            let mut pick = SupportStatus::Verified;
            for child in multileader.display() {
                let (r, p) = display_support(child);
                render = weaker(render, r);
                pick = weaker(pick, p);
            }
            (render, pick)
        }
        _ => (SupportStatus::Verified, SupportStatus::Verified),
    }
}

/// Whether a hatch's derived display carries the fill/pattern its definition
/// asks for.
///
/// A solid/gradient hatch whose fill failed (or whose gradient kind is outside
/// the implemented subset) shows boundaries only; a pattern hatch whose families
/// are empty has nothing but its boundary loops. Both are lossy and must not be
/// reported `Verified`.
fn hatch_display_is_complete(hatch: &HatchGeometry) -> bool {
    match hatch.fill() {
        HatchFill::Solid | HatchFill::Gradient(_) => hatch
            .display()
            .iter()
            .any(|geometry| matches!(geometry, SemanticGeometry::Mesh(_))),
        HatchFill::Pattern(_) => {
            let polylines = hatch
                .display()
                .iter()
                .filter(|geometry| matches!(geometry, SemanticGeometry::Polyline { .. }))
                .count();
            polylines > hatch.loops().len()
        }
    }
}
/// Whether a proxy cache may contribute geometry for this entity.
///
/// Only entity types with no semantic representation of their own are expanded
/// from `graphic_data`. A known entity (LINE, HATCH, ...) is drawn from its
/// semantic geometry, so overlaying its proxy cache would draw it twice
/// (audit B21; `docs/proxy-support.md` §4).
pub(crate) fn proxy_geometry_allowed(entity: &EntityType) -> bool {
    matches!(entity, EntityType::Unknown(_))
        || matches!(entity, EntityType::Extended(x) if x.class_name() == "ACAD_PROXY_ENTITY")
}

/// Fold every geometry item decoded from one proxy cache into a single
/// semantic geometry without dropping any.
///
/// One item stays itself; two or more become a [`SemanticGeometry::Compound`]
/// so the representation layer emits one drawable per item. This replaces the
/// previous `.into_iter().next()`, which silently discarded all but the first
/// fragment (audit B21).
pub(crate) fn proxy_geometry_compound(mut geometry: Vec<SemanticGeometry>) -> SemanticGeometry {
    match geometry.len() {
        1 => geometry.pop().expect("length checked"),
        _ => SemanticGeometry::Compound(geometry),
    }
}
/// Combine import-stage problems with the render support of model content.
///
/// `Complete` only when everything in model space can be drawn and the stream
/// was whole; otherwise `Partial`, or `Missing` when nothing at all is
/// drawable and there was no separate import fault to report.
pub(crate) fn aggregate_completeness(
    model_render: SupportStatus,
    model_drawable: bool,
    mut model_render_types: Vec<String>,
    import_items: Vec<String>,
) -> Completeness {
    let mut items = import_items;
    let has_import_item = !items.is_empty();
    if model_render != SupportStatus::Verified {
        model_render_types.sort();
        // Covers both entities with no representation at all (Opaque/ACIS) and
        // entities whose representation is only conditionally drawable (text
        // needs a host font). Neither is a `Verified` claim.
        items.push(format!(
            "display representation not verified for: {}",
            model_render_types.join(", ")
        ));
    }
    if items.is_empty() {
        Completeness::Complete
    } else if model_drawable || has_import_item {
        Completeness::Partial(items)
    } else {
        Completeness::Missing(items)
    }
}
pub(crate) fn weaker(a: SupportStatus, b: SupportStatus) -> SupportStatus {
    let rank = |s: &SupportStatus| match s {
        SupportStatus::Verified => 4,
        SupportStatus::Partial => 3,
        SupportStatus::Unverified => 2,
        SupportStatus::Unsupported => 1,
        SupportStatus::NotImplemented => 0,
    };
    match (rank(&a), rank(&b)) {
        (x, y) if x <= y => a,
        _ => b,
    }
}
pub(crate) fn entity_class_name(e: &EntityType) -> String {
    match e {
        EntityType::Line(_) => "AcDbLine",
        EntityType::Circle(_) => "AcDbCircle",
        EntityType::Arc(_) => "AcDbArc",
        EntityType::Ellipse(_) => "AcDbEllipse",
        EntityType::Point(_) => "AcDbPoint",
        EntityType::LwPolyline(_) => "AcDbPolyline",
        EntityType::Polyline2D(_) => "AcDb2dPolyline",
        EntityType::Polyline3D(_) => "AcDb3dPolyline",
        EntityType::Text(_) => "AcDbText",
        EntityType::MText(_) => "AcDbMText",
        EntityType::Spline(_) => "AcDbSpline",
        EntityType::Solid(_) => "AcDbTrace",
        EntityType::Face3D(_) => "AcDbFace",
        EntityType::Insert(_) => "AcDbBlockReference",
        EntityType::Solid3D(_) => "AcDb3dSolid",
        EntityType::Region(_) => "AcDbRegion",
        EntityType::Body(_) => "AcDbBody",
        EntityType::Surface(_) => "AcDbSurface",
        EntityType::Hatch(_) => "AcDbHatch",
        EntityType::Dimension(_) => "AcDbDimension",
        EntityType::Leader(_) => "AcDbLeader",
        EntityType::MultiLeader(_) => "AcDbMultiLeader",
        EntityType::Polyline(_) => "AcDbPolyline",
        EntityType::AttributeDefinition(_) => "AcDbAttributeDefinition",
        EntityType::AttributeEntity(_) => "AcDbAttribute",
        EntityType::Mesh(_) => "AcDbSubDMesh",
        EntityType::PolyfaceMesh(_) => "AcDbPolyFaceMesh",
        EntityType::PolygonMesh(_) => "AcDbPolygonMesh",
        EntityType::Wipeout(_) => "AcDbWipeout",
        EntityType::Helix(_) => "AcDbHelix",
        EntityType::Ray(_) => "AcDbRay",
        EntityType::XLine(_) => "AcDbXline",
        EntityType::Viewport(_) => "AcDbViewport",
        EntityType::Tolerance(_) => "AcDbFcf",
        EntityType::MLine(_) => "AcDbMline",
        EntityType::Shape(_) => "AcDbShape",
        EntityType::Table(_) => "AcDbTable",
        EntityType::RasterImage(_) => "AcDbRasterImage",
        EntityType::Ole2Frame(_) => "AcDbOle2Frame",
        EntityType::ViewBorder(_) => "AcDbViewBorder",
        EntityType::SectionSymbol(_) => "AcDbSectionSymbol",
        EntityType::Underlay(_) => "AcDbUnderlayReference",
        EntityType::Light(_) => "AcDbLight",
        EntityType::Unknown(u) => return u.dxf_name.clone(),
        EntityType::Extended(x) => return x.class_name().to_string(),
        other => {
            return format!("{other:?}")
                .split('(')
                .next()
                .unwrap_or("Unknown")
                .to_string()
        }
    }
    .to_string()
}
/// A DWG file starts with an `AC10xx` version signature.
pub(crate) fn looks_like_dwg(bytes: &[u8]) -> bool {
    bytes.len() >= 6 && &bytes[0..2] == b"AC" && bytes[2..6].iter().all(|b| b.is_ascii_digit())
}

/// True for the model/paper space records rather than a user block.
///
/// DWG version differences use mixed case (`*Model_Space`) and upper case
/// (`*MODEL_SPACE`, `*PAPER_SPACE`); matching only one casing imports the model
/// space twice (audit: R14 files).
pub(crate) fn is_space_block_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper == "*MODEL_SPACE" || upper.starts_with("*PAPER_SPACE")
}

pub(crate) fn is_paper_space_name(name: &str) -> bool {
    name.to_ascii_uppercase().starts_with("*PAPER_SPACE")
}

pub(crate) fn compute_identity(bytes: &[u8]) -> DocumentIdentity {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    DocumentIdentity::Sha256(out)
}
pub(crate) trait HandleExt {
    fn null_or_value_zero(&self) -> bool;
}

impl HandleExt for acadrust::Handle {
    fn null_or_value_zero(&self) -> bool {
        self.value() == 0
    }
}

#[cfg(test)]
mod dimension_support_tests {
    use super::*;
    use cad_domain::{
        DimensionDefinition, DimensionGeometry, DimensionKind, DimensionPoints,
        DimensionStyleValues,
    };

    fn style() -> DimensionStyleValues {
        DimensionStyleValues {
            arrow_size: 2.5,
            ext_offset: 0.625,
            ext_beyond: 1.25,
            text_height: 2.5,
            text_gap: 0.625,
            center_mark: 0.0,
            dim_line_increment: 3.75,
            text_vertical: 1,
            text_vertical_position: 0.0,
            suppress_ext1: false,
            suppress_ext2: false,
            suppress_dim1: false,
            suppress_dim2: false,
            linear_factor: 1.0,
            decimals: 4,
            rounding: 0.0,
            postfix: String::new(),
            zero_suppression: 0,
            linear_unit_format: 2,
            decimal_separator: 46,
            text_style_name: "Standard".into(),
            text_style: StyleId(0),
            text_font: None,
        }
    }

    fn definition() -> DimensionDefinition {
        DimensionDefinition {
            kind: DimensionKind::Aligned,
            points: DimensionPoints::Linear {
                first: Point3 {
                    x: -1.0,
                    y: 0.0,
                    z: 0.0,
                },
                second: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                dimension_line: Point3 {
                    x: -1.0,
                    y: -2.0,
                    z: 0.0,
                },
            },
            text_middle: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            text_rotation: 0.0,
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        }
    }

    /// ADR 0017 F.3 (wildcard regression): a dimension with a Text child must be
    /// `Unverified`, not the false `Verified` the wildcard arm would give it.
    #[test]
    fn dimension_with_text_child_is_unverified() {
        let dimension =
            DimensionGeometry::synthesized(definition(), "Standard".into(), style(), None)
                .expect("synthesized");
        let (render, pick) = display_support(&SemanticGeometry::Dimension(Box::new(dimension)));
        assert_eq!(render, SupportStatus::Unverified);
        assert_eq!(pick, SupportStatus::Unverified);
    }
}

#[cfg(test)]
mod hatch_support_tests {
    use super::*;

    fn rect_path() -> acadrust::entities::BoundaryPath {
        let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let mut path = acadrust::entities::BoundaryPath::new();
        for i in 0..4 {
            let a = corners[i];
            let b = corners[(i + 1) % 4];
            path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
                start: acadrust::types::Vector2::new(a.0, a.1),
                end: acadrust::types::Vector2::new(b.0, b.1),
            }));
        }
        path
    }

    fn solid_hatch() -> SemanticGeometry {
        let mut hatch = acadrust::entities::Hatch::new();
        hatch.is_solid = true;
        hatch.paths = vec![rect_path()];
        ImporterBuilder::hatch_geometry(&hatch).0
    }

    fn boundary_only_pattern() -> SemanticGeometry {
        // A user-defined pattern with zero spacing cannot be synthesized, so the
        // display is boundary-only; reporting it `Verified` would over-claim.
        let mut hatch = acadrust::entities::Hatch::new();
        hatch.is_solid = false;
        hatch.pattern_type = acadrust::entities::HatchPatternType::UserDefined;
        hatch.pattern.name = "USER".into();
        hatch.pattern_scale = 0.0;
        hatch.paths = vec![rect_path()];
        ImporterBuilder::hatch_geometry(&hatch).0
    }

    #[test]
    fn complete_hatch_display_is_verified() {
        assert_eq!(
            display_support(&solid_hatch()),
            (SupportStatus::Verified, SupportStatus::Verified)
        );
    }

    #[test]
    fn boundary_only_hatch_is_not_reported_verified() {
        let (render, pick) = display_support(&boundary_only_pattern());
        assert_eq!(render, SupportStatus::Partial);
        assert_eq!(pick, SupportStatus::Partial);
    }
}
