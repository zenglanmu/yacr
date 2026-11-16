//! The object-type table of the select/filter batch: the stable friendly
//! geometry keys (`LINE`, `CIRCLE`, …) and the AutoCAD type aliases
//! (`TEXT`/`MTEXT`, `MASK`/`WIPEOUT`, and the body source classes
//! `SOLID`/`SURFACE`/`REGION`/`BODY`).
//!
//! Split out of `select_filter.rs` to keep that file inside the code-size limit.
//! [`type_matches`] and [`geometry_key`] are re-exported there, so the public
//! paths (`cad_app::select_filter::type_matches`,
//! `cad_app::select_filter::geometry_key`) are unchanged.

use cad_db::DbEntity;
use cad_domain::SemanticGeometry;

/// Every type name one entity answers to: the stored type key plus the friendly
/// geometry alias (`MTEXT`/`TEXT`, `WIPEOUT`, `LINE`, …).
pub(super) fn type_names(entity: &DbEntity) -> Vec<String> {
    let mut names = vec![entity.object.type_key.clone()];
    match &entity.geometry {
        SemanticGeometry::Text { .. } => {
            if entity.object.type_key.eq_ignore_ascii_case("AcDbMText") {
                names.push("MTEXT".into());
            } else {
                names.push("TEXT".into());
            }
        }
        SemanticGeometry::Mask { .. } => names.push("WIPEOUT".into()),
        other => names.push(geometry_key(other).to_string()),
    }
    names
}

/// Whether an entity's stored type key **or** friendly geometry alias matches
/// `value` (ASCII case-insensitive).
///
/// `MTEXT` is a friendly alias for a `Text` geometry stamped `AcDbMText`; the
/// other aliases come from the geometry discriminant. A `Mask` matches both
/// `MASK` and its AutoCAD name `WIPEOUT`.
pub fn type_matches(entity: &DbEntity, value: &str) -> bool {
    if entity.object.type_key.eq_ignore_ascii_case(value) {
        return true;
    }
    match &entity.geometry {
        // TEXT and MTEXT are the same geometry variant; the stored type key is
        // the only distinction, so each alias names exactly one source class.
        SemanticGeometry::Text { .. } => {
            if entity.object.type_key.eq_ignore_ascii_case("AcDbMText") {
                value.eq_ignore_ascii_case("MTEXT")
            } else {
                value.eq_ignore_ascii_case("TEXT")
            }
        }
        SemanticGeometry::Mask { .. } => {
            value.eq_ignore_ascii_case("MASK") || value.eq_ignore_ascii_case("WIPEOUT")
        }
        // A body's source class is only in the stored `type_key`: the geometry
        // variant cannot tell a solid from a surface or a planar REGION.
        SemanticGeometry::Body3d(_) => body_type_matches(&entity.object.type_key, value),
        other => geometry_key(other).eq_ignore_ascii_case(value),
    }
}

/// Whether a body's stored source class answers to the filter value `value`.
///
/// The mapping is from the entity's own `type_key` to AutoCAD's type name (the
/// geometry variant cannot tell a solid from a surface or a planar REGION); an
/// unknown/vendor body class answers only the neutral `BODY` alias.
fn body_type_matches(type_key: &str, value: &str) -> bool {
    let alias = match () {
        _ if type_key.eq_ignore_ascii_case("AcDb3dSolid") => "SOLID",
        _ if type_key.eq_ignore_ascii_case("AcDbSurface") => "SURFACE",
        _ if type_key.eq_ignore_ascii_case("AcDbRegion") => "REGION",
        _ => "BODY",
    };
    value.eq_ignore_ascii_case(alias) || value.eq_ignore_ascii_case("BODY")
}

/// The stable friendly name of one geometry discriminant (`LINE`, `CIRCLE`, …).
pub fn geometry_key(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "LINE",
        SemanticGeometry::Polyline { .. } => "POLYLINE",
        SemanticGeometry::Circle { .. } => "CIRCLE",
        SemanticGeometry::Arc { .. } => "ARC",
        SemanticGeometry::Ellipse { .. } => "ELLIPSE",
        SemanticGeometry::Spline { .. } => "SPLINE",
        SemanticGeometry::Point(_) => "POINT",
        SemanticGeometry::Mesh(_) => "MESH",
        SemanticGeometry::Insert { .. } => "INSERT",
        SemanticGeometry::Text { .. } => "TEXT",
        SemanticGeometry::Shape { .. } => "SHAPE",
        SemanticGeometry::Opaque { .. } => "OPAQUE",
        SemanticGeometry::Compound(_) => "COMPOUND",
        SemanticGeometry::Dimension(_) => "DIMENSION",
        SemanticGeometry::Hatch(_) => "HATCH",
        SemanticGeometry::MultiLeader(_) => "MULTILEADER",
        SemanticGeometry::Table(_) => "TABLE",
        SemanticGeometry::Image { .. } => "IMAGE",
        SemanticGeometry::Mask { .. } => "MASK",
        // A body's source class is not derivable from the geometry variant; this
        // is the neutral geometry alias, and `type_matches` reads the type key.
        SemanticGeometry::Body3d(_) => "BODY",
    }
}
