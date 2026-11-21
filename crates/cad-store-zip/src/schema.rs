//! `schema/classes.json`: the class registry, materialised at save time.
//!
//! The registry is compiled in (`cad_domain::ymodel::descriptor::CLASSES`), and
//! ADR-0037 D10 asks for it to be written into the archive so a reader can decide
//! what an element payload means without linking this build. There is no EC
//! interpreter and no reflection: this module only names each descriptor field in
//! the file.
//!
//! ```text
//! {"classes": [
//!   {"fields": [{"kind": "point3", "name": "start"}, {"kind": "point3", "name": "end"}],
//!    "id": {"ordinal": 1, "schema": 1},
//!    "type_key": "cad:Line"},
//!   …]}
//! ```
//!
//! Two rules make this stable:
//!
//! * the order is `CLASSES` order (schema, then ordinal), which is the format
//!   order — never renumbered (D10);
//! * the spelling of every [`FieldKind`] is pinned here. The match is
//!   exhaustive, so a new field kind cannot be added without deciding its
//!   spelling in the same commit; `FieldKind` is not `#[non_exhaustive]`, so the
//!   compiler is the gate. `IdRef` carries its family in the token
//!   (`"id:layer"`), which is why the spelling is an owned `String`.

use serde_json::{Map, Value};

use cad_domain::ymodel::descriptor::{ClassDescriptor, FieldKind};
use cad_domain::ymodel::{IdKind, CLASSES};

/// The `schema/classes.json` object.
pub(crate) fn classes_value() -> Value {
    let classes: Vec<Value> = CLASSES.iter().map(class_value).collect();
    let mut map = Map::new();
    map.insert("classes".to_string(), Value::Array(classes));
    Value::Object(map)
}

fn class_value(class: &ClassDescriptor) -> Value {
    let fields: Vec<Value> = class
        .fields
        .iter()
        .map(|field| {
            let mut entry = Map::new();
            entry.insert("name".to_string(), Value::String(field.name.to_string()));
            entry.insert(
                "kind".to_string(),
                Value::String(field_kind_spelling(field.kind)),
            );
            Value::Object(entry)
        })
        .collect();

    let mut id = Map::new();
    id.insert("schema".to_string(), Value::from(class.id.schema().0));
    id.insert("ordinal".to_string(), Value::from(class.id.ordinal()));

    let mut map = Map::new();
    map.insert("id".to_string(), Value::Object(id));
    map.insert(
        "type_key".to_string(),
        Value::String(class.type_key.to_string()),
    );
    map.insert("fields".to_string(), Value::Array(fields));
    Value::Object(map)
}

/// The token a field kind is written as (snake_case of the variant name).
fn field_kind_spelling(kind: FieldKind) -> String {
    let fixed = match kind {
        FieldKind::F64 => "f64",
        FieldKind::OptF64 => "opt_f64",
        FieldKind::U32 => "u32",
        FieldKind::Bool => "bool",
        FieldKind::String => "string",
        FieldKind::OptString => "opt_string",
        FieldKind::Point3 => "point3",
        FieldKind::VecPoint3 => "vec_point3",
        FieldKind::VecF64 => "vec_f64",
        FieldKind::F64x2 => "f64x2",
        FieldKind::VecF64x2 => "vec_f64x2",
        FieldKind::VecU32x3 => "vec_u32x3",
        FieldKind::U8x3List => "u8x3_list",
        FieldKind::VecOptSubElement => "vec_opt_sub_element",
        FieldKind::MeshPbr => "mesh_pbr",
        FieldKind::Geometry => "geometry",
        FieldKind::VecGeometry => "vec_geometry",
        FieldKind::Bytes => "bytes",
        FieldKind::Transform3 => "transform3",
        FieldKind::TextAlignH => "text_align_h",
        FieldKind::TextAlignV => "text_align_v",
        FieldKind::TextLineSpacing => "text_line_spacing",
        FieldKind::BodyKind => "body_kind",
        FieldKind::BodyExact => "body_exact",
        FieldKind::BodyDisplay => "body_display",
        FieldKind::DimensionDefinition => "dimension_definition",
        FieldKind::DimensionStyleValues => "dimension_style_values",
        FieldKind::DisplaySource => "display_source",
        FieldKind::HatchLoops => "hatch_loops",
        FieldKind::HatchPlane => "hatch_plane",
        FieldKind::HatchStyle => "hatch_style",
        FieldKind::HatchFill => "hatch_fill",
        FieldKind::HatchPatternType => "hatch_pattern_type",
        FieldKind::MLeaderStyleValues => "mleader_style_values",
        FieldKind::MLeaderContent => "mleader_content",
        FieldKind::MLeaderText => "mleader_text",
        FieldKind::MLeaderRoots => "mleader_roots",
        FieldKind::MLeaderLines => "mleader_lines",
        FieldKind::MLeaderReserved => "mleader_reserved",
        FieldKind::PlaneFrame => "plane_frame",
        FieldKind::TableFrame => "table_frame",
        FieldKind::TableColumns => "table_columns",
        FieldKind::TableRows => "table_rows",
        FieldKind::TableStyleValues => "table_style_values",
        FieldKind::TableMargins => "table_margins",
        FieldKind::TableDisplay => "table_display",
        FieldKind::OptImageClip => "opt_image_clip",
        FieldKind::ImageAdjust => "image_adjust",
        FieldKind::Space => "space",
        FieldKind::RenderAttributes => "render_attributes",
        // ADR-0042 D2: `cad-db` owns the nested shape; this file only names it.
        FieldKind::Bim => "bim",
        FieldKind::IdRef(kind) => return format!("id:{}", id_kind_spelling(kind)),
    };
    fixed.to_string()
}

/// The token an identifier family is written as.
///
/// The numeric values are the format constants (see `cad_domain::ymodel::id`);
/// this is their human-readable spelling, used only inside a field kind and in
/// diagnostics, never as a stand-in for the number.
fn id_kind_spelling(kind: IdKind) -> &'static str {
    match kind {
        IdKind::Entity => "entity",
        IdKind::Layer => "layer",
        IdKind::Block => "block",
        IdKind::Layout => "layout",
        IdKind::Style => "style",
        IdKind::DimStyle => "dim_style",
        IdKind::MLeaderStyle => "mleader_style",
        IdKind::Linetype => "linetype",
        IdKind::Scale => "scale",
        IdKind::ModelSpace => "model_space",
        IdKind::SubCategory => "sub_category",
        IdKind::Relationship => "relationship",
        IdKind::Aspect => "aspect",
        IdKind::PaperModel => "paper_model",
        IdKind::BlockModel => "block_model",
        IdKind::DefinitionModel => "definition_model",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_class_and_field_is_written_once_in_format_order() {
        let value = classes_value();
        let classes = value["classes"].as_array().expect("classes array");
        assert_eq!(classes.len(), CLASSES.len());
        let mut ids = BTreeSet::new();
        for (index, class) in classes.iter().enumerate() {
            let source = &CLASSES[index];
            assert_eq!(class["type_key"], Value::String(source.type_key.into()));
            assert_eq!(
                class["id"]["schema"].as_u64(),
                Some(u64::from(source.id.schema().0))
            );
            assert_eq!(
                class["id"]["ordinal"].as_u64(),
                Some(u64::from(source.id.ordinal()))
            );
            let id = class["id"]["schema"].as_u64().unwrap() << 32
                | class["id"]["ordinal"].as_u64().unwrap();
            assert!(ids.insert(id), "class ids are unique");
            let fields = class["fields"].as_array().expect("fields array");
            assert_eq!(fields.len(), source.fields.len());
            for (field, descriptor) in fields.iter().zip(source.fields) {
                assert_eq!(field["name"], Value::String(descriptor.name.into()));
                assert_eq!(
                    field["kind"],
                    Value::String(field_kind_spelling(descriptor.kind))
                );
            }
        }
    }

    #[test]
    fn identifier_fields_name_their_family() {
        assert_eq!(
            field_kind_spelling(FieldKind::IdRef(IdKind::Layer)),
            "id:layer"
        );
        assert_eq!(
            field_kind_spelling(FieldKind::IdRef(IdKind::Block)),
            "id:block"
        );
        // A non-identifier kind never borrows the family syntax.
        assert!(!field_kind_spelling(FieldKind::Point3).contains(':'));
    }

    #[test]
    fn the_mesh_pbr_field_kind_is_published() {
        // ADR-0039 D2: every `FieldKind` variant must be spellable here, and the
        // registry must actually use this one on `cad:Mesh`.
        assert_eq!(field_kind_spelling(FieldKind::MeshPbr), "mesh_pbr");
        let mesh = cad_domain::ymodel::by_type_key("cad:Mesh").expect("cad:Mesh is registered");
        assert!(
            mesh.fields
                .iter()
                .any(|field| field.name == "pbr" && field.kind == FieldKind::MeshPbr),
            "cad:Mesh must declare the pbr field"
        );
    }

    #[test]
    fn the_object_is_deterministic() {
        let first = serde_json::to_vec(&classes_value()).expect("encode");
        let second = serde_json::to_vec(&classes_value()).expect("encode");
        assert_eq!(first, second);
        // Key order is NOT asserted: `serde_json::Map` is a `BTreeMap` only when
        // no crate in the build enables `preserve_order`, which a transitive
        // dependency does. Both orders are the same JSON object, and nothing in
        // the format or the reader depends on it.
        let text = String::from_utf8(first).expect("utf8");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(
            value.as_object().map(|object| object.len()),
            Some(1),
            "the registry object has exactly one top-level key: {text}"
        );
        let classes = value["classes"].as_array().expect("an array");
        assert!(!classes.is_empty());
        for class in classes {
            let object = class.as_object().expect("each class is an object");
            for key in ["id", "type_key", "fields"] {
                assert!(object.contains_key(key), "class is missing {key}: {text}");
            }
        }
    }
}
