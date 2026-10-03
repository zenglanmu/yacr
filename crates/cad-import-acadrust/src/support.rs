//! Support classification, proxy policy and document identity.

use super::*;

/// Retain a solid/surface entity's ACIS payload as opaque bytes.
///
/// The neutral lift for tessellation is [`solid_exchange_from_entity`]; this
/// opaque form keeps the raw SAT/SAB for provenance and for a later kernel
/// provider, and deliberately claims no display support of its own.
pub(crate) fn acis_semantics(
    entity: &EntityType,
    acis: &acadrust::entities::AcisData,
) -> (SemanticGeometry, Completeness) {
    let (version, payload) = acis_raw_payload(acis);
    (
        SemanticGeometry::Opaque {
            type_key: entity_class_name(entity),
            version,
            payload,
        },
        Completeness::Unverified,
    )
}

/// Whether the display pipeline can actually draw this geometry (audit B20).
///
/// Text has no scene batching yet and opaque/ACIS geometry has no
/// representation, so neither is `Verified`; an INSERT depends on its block
/// contents and is resolved by the caller.
pub(crate) fn display_support(geometry: &SemanticGeometry) -> (SupportStatus, SupportStatus) {
    match geometry {
        SemanticGeometry::Opaque { .. } => (SupportStatus::Unsupported, SupportStatus::Unsupported),
        // Any named font is host-resolvable: the host registers arbitrary
        // `name=path` keys and legacy DXF names such as `txt` omit the `.shx`
        // extension, so a name mismatch is a font-availability question, not a
        // missing representation. Only a text with no font name at all is
        // `Unsupported`.
        SemanticGeometry::Text { font, .. } => match font.as_deref() {
            Some(name) if !name.trim().is_empty() => {
                (SupportStatus::Unverified, SupportStatus::Unverified)
            }
            _ => (SupportStatus::Unsupported, SupportStatus::Unsupported),
        },
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
        _ => (SupportStatus::Verified, SupportStatus::Verified),
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
