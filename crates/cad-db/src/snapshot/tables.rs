//! Tables row codecs: `serde_json::Value` ⇄ rows (crate-private; only
//! [`super::DocumentSnapshot`] crosses the store boundary).
//!
//! Contract: one object per row; **every** identifier, including ids referenced
//! from a row, is a persistent composite id (a bare sequence collides across
//! families); every `f64` goes through `ymodel::json::{encode_f64, decode_f64}`
//! so legacy non-finite values survive; enum variants are `{"kind": "<tag>"}`
//! objects with kebab-case names, except [`ExternalReferenceStatus`], whose
//! tags are its stable `as_str()` spellings.
//! Decoding is strict both ways: a missing or extra field, a wrong type, an id
//! of the wrong family or an unknown tag is refused with
//! `SnapshotError::invalid`, naming the table and row id.
//!
//! `Layer::visible` is derived (`!(off || frozen)`, like `drawing::layer_edit`)
//! and style name indexes are rebuilt by the caller. `block_name_order` *is*
//! persisted: last-duplicate-wins lookup cannot be rebuilt from a map keyed by
//! id (see `encode_blocks`). An absent table decodes to an empty map;
//! `Layouts` never contains `LayoutId(0)`. The body is hand-packed to fit the
//! 1000-line file gate and is `rustfmt::skip`ped as a unit.
use super::{table_entry, SnapshotError, TableSnapshot};
use crate::dim_style::DimStyle;
use crate::drawing::DrawingDatabase;
use crate::entity::LinetypePattern;
use crate::mleader_style::MLeaderStyle;
use crate::scope::TableKind;
use crate::tables::{
    BlockDefinition, BlockMetadata, DynamicBlockState, DynamicBlockVisibility, Layer,
    LayerAttributes, LayerColor, LayerLineWeight, Layout, LineType, PaperViewport, PlotMargins,
    PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord, PlotType, Scale, Style,
};
use crate::xref::{ExternalReference, ExternalReferenceStatus};
use cad_domain::ymodel::json::{decode_f64, decode_point3, encode_f64, encode_point3};
use cad_domain::ymodel::{PersistentId, PersistentIdOf};
use cad_domain::{
    BlockId, Completeness, DimStyleId, EntityId, LayerId, LayoutId, LinetypeId, MLeaderPathType,
    MLeaderStyleId, MLeaderTextAngle, Point3, ScaleId, StyleId, Transform3,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
#[rustfmt::skip]
mod codec {
    use super::*;
    /// The decoded tables, mirroring the [`DrawingDatabase`] side tables one for
    /// one (the two layer maps stay separate though they share a row).
    #[derive(Debug, Clone, Default, PartialEq)]
    pub(crate) struct DecodedTables {
        pub(crate) layers: BTreeMap<LayerId, Layer>,
        pub(crate) layer_attributes: BTreeMap<LayerId, LayerAttributes>,
        pub(crate) blocks: BTreeMap<BlockId, BlockDefinition>,
        pub(crate) block_metadata: BTreeMap<BlockId, BlockMetadata>,
        pub(crate) block_name_order: Vec<(String, BlockId)>,
        pub(crate) block_source_handles: BTreeMap<u64, BlockId>,
        pub(crate) layouts: BTreeMap<LayoutId, Layout>,
        pub(crate) plot_settings: BTreeMap<LayoutId, PlotSettingsRecord>,
        pub(crate) styles: BTreeMap<StyleId, Style>,
        pub(crate) dim_styles: BTreeMap<DimStyleId, DimStyle>,
        pub(crate) mleader_styles: BTreeMap<MLeaderStyleId, MLeaderStyle>,
        pub(crate) linetypes: BTreeMap<LinetypeId, LineType>,
        pub(crate) scales: BTreeMap<ScaleId, Scale>,
        pub(crate) external_references: BTreeMap<String, ExternalReference>,
    }
    /// Build one row object; keys serialize sorted (the canonical order).
    macro_rules! row {
        ($($key:literal => $value:expr),* $(,)?) => {{
            #[allow(unused_mut)]
            let mut map = Map::new();
            $(map.insert($key.to_string(), $value);)*
            Value::Object(map)
        }};
    }
    /// Encode every row table. All ten are written, including empty ones:
    /// Phase 1 rewrites the whole archive, so a reader never guesses whether a
    /// missing table means "not written" or "not modelled".
    pub(crate) fn encode(db: &DrawingDatabase) -> Result<TableSnapshot, SnapshotError> {
        let mut tables = TableSnapshot::default();
        tables.set(TableKind::Layers, encode_layers(db)?);
        tables.set(TableKind::Blocks, encode_blocks(db)?);
        tables.set(TableKind::Layouts, encode_layouts(db));
        tables.set(TableKind::PlotSettings, encode_plot_settings(db));
        tables.set(TableKind::Styles, encode_styles(db));
        tables.set(TableKind::DimStyles, encode_dim_styles(db));
        tables.set(TableKind::MLeaderStyles, encode_mleader_styles(db));
        tables.set(TableKind::Linetypes, encode_linetypes(db));
        tables.set(TableKind::Scales, encode_scales(db));
        tables.set(TableKind::ExternalReferences, encode_xrefs(db));
        Ok(tables)
    }
    fn encode_layers(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
        // The two in-memory maps are one row on disk, so an attribute row
        // without a layer row has nowhere to go; no writer here can produce it.
        if let Some(id) = db.layer_attributes.keys().find(|id| !db.layers.contains_key(id)) {
            return Err(SnapshotError::invalid(format!(
                "{}: layer attributes for unknown layer {}",
                table_entry(TableKind::Layers), id.0)));
        }
        Ok(db.layers.values().map(|layer| {
            // A missing attribute row resolves to the documented default
            // everywhere, so that is what the row carries; `visible` is then
            // recomputed from it on decode.
            let attrs = db.layer_attributes.get(&layer.id).cloned().unwrap_or_default();
            row! { "id" => pid(&layer.id), "name" => text(&layer.name),
                   "attributes" => encode_layer_attributes(&attrs) }
        }).collect())
    }
    fn encode_layer_attributes(a: &LayerAttributes) -> Value {
        row! {
            "off" => Value::Bool(a.off), "frozen" => Value::Bool(a.frozen),
            "frozen_in_new_viewport" => Value::Bool(a.frozen_in_new_viewport),
            "locked" => Value::Bool(a.locked), "xref_dependent" => Value::Bool(a.xref_dependent),
            "plottable" => Value::Bool(a.plottable),
            "color" => row! { "aci" => a.color.aci.map_or(Value::Null, Value::from),
                              "rgb" => bytes(&a.color.rgb) },
            "linetype" => text(&a.linetype), "lineweight" => encode_line_weight(a.lineweight),
            "opacity" => encode_f64(f64::from(a.opacity)),
            "plot_style" => a.plot_style.as_deref().map_or(Value::Null, Value::from),
        }
    }
    fn encode_line_weight(weight: LayerLineWeight) -> Value {
        match weight {
            LayerLineWeight::Hundredths(value) => row! {
                "kind" => text("hundredths"), "value" => Value::from(value) },
            LayerLineWeight::Default => kind_tag("default"),
            LayerLineWeight::ByLayer => kind_tag("by-layer"),
        }
    }
    fn encode_blocks(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
        // A registration whose block row is gone would be dropped, changing
        // last-duplicate-wins resolution; refuse it instead.
        if let Some((name, id)) = db.block_name_order.iter()
            .find(|(_, id)| !db.blocks.contains_key(id)) {
            return Err(SnapshotError::invalid(format!("{}: name registration {name:?} points \
                at unknown block {}", table_entry(TableKind::Blocks), id.0)));
        }
        // `registration_index` is the persisted position in `block_name_order`;
        // unlisted blocks continue after the listed ones in id order. Persisted
        // rather than derived because `block_by_name` returns the **last
        // registered** duplicate, which a map keyed by id cannot reproduce.
        let mut next_index = db.block_name_order.len() as u32;
        let mut encoded = Vec::with_capacity(db.blocks.len());
        for (id, block) in &db.blocks {
            let registration_index = db.block_name_order.iter()
                .position(|(_, registered)| registered == id).map(|p| p as u32)
                .unwrap_or_else(|| { let i = next_index; next_index += 1; i });
            let metadata = db.block_metadata.get(id);
            // One row carries one handle; the lowest wins if two were recorded.
            let handle = db.block_source_handles.iter()
                .find(|(_, owner)| *owner == id).map(|(handle, _)| *handle);
            encoded.push(row! { "id" => pid(id),
                "name" => metadata.map_or(Value::Null, |m| text(&m.name)),
                "base_point" => metadata.map_or(Value::Null, |m| encode_point3(m.base_point)),
                "entities" => entity_ids(&block.entities),
                "dynamic_visibility" => block.dynamic_visibility.as_ref()
                    .map_or(Value::Null, encode_dynamic_visibility),
                "source_handle" => handle.map_or(Value::Null, Value::from),
                "registration_index" => Value::from(registration_index) });
        }
        Ok(encoded)
    }
    fn encode_dynamic_visibility(visibility: &DynamicBlockVisibility) -> Value {
        row! {
            "member_entities" => entity_ids(&visibility.member_entities),
            "states" => Value::Array(visibility.states.iter().map(|state| row! {
                "name" => text(&state.name), "entities" => entity_ids(&state.entities)
            }).collect()),
            "active_state" => visibility.active_state.as_deref().map_or(Value::Null, Value::from),
        }
    }
    fn encode_layouts(db: &DrawingDatabase) -> Vec<Value> {
        // `LayoutId(0)` is synthetic model space, not a stored layout.
        db.layouts.values().filter(|layout| layout.id.0 != 0).map(|layout| {
            row! { "id" => pid(&layout.id), "name" => text(&layout.name),
                   "viewports" => Value::Array(
                       layout.viewports.iter().map(encode_viewport).collect()) }
        }).collect()
    }
    fn encode_viewport(viewport: &PaperViewport) -> Value {
        row! {
            "clip" => Value::Array(viewport.clip.iter().map(|p| encode_point3(*p)).collect()),
            "model_to_paper" => encode_transform3(&viewport.model_to_paper),
            "completeness" => encode_completeness(&viewport.completeness),
            "layer_overrides" => Value::Array(viewport.layer_overrides.iter()
                .map(|(layer, frozen)| row! {
                    "layer" => pid(layer), "frozen" => Value::Bool(*frozen) }).collect()),
        }
    }
    fn encode_transform3(transform: &Transform3) -> Value {
        Value::Array(transform.matrix.iter().map(|line| f64s(line)).collect())
    }
    fn encode_completeness(completeness: &Completeness) -> Value {
        match completeness {
            Completeness::Complete => kind_tag("complete"),
            Completeness::Unverified => kind_tag("unverified"),
            Completeness::Partial(reasons) => row! {
                "kind" => text("partial"), "reasons" => strings(reasons) },
            Completeness::Missing(reasons) => row! {
                "kind" => text("missing"), "reasons" => strings(reasons) },
        }
    }
    fn encode_plot_settings(db: &DrawingDatabase) -> Vec<Value> {
        db.plot_settings.values().map(|s| row! {
            "layout" => pid(&s.layout), "paper_size_name" => text(&s.paper_size_name),
            "paper_width" => encode_f64(s.paper_width),
            "paper_height" => encode_f64(s.paper_height),
            "margins" => row! { "left" => encode_f64(s.margins.left),
                "bottom" => encode_f64(s.margins.bottom),
                "right" => encode_f64(s.margins.right), "top" => encode_f64(s.margins.top) },
            "rotation" => kind_tag(match s.rotation {
                PlotRotation::None => "none", PlotRotation::Degrees90 => "degrees-90",
                PlotRotation::Degrees180 => "degrees-180",
                PlotRotation::Degrees270 => "degrees-270" }),
            "scale_numerator" => encode_f64(s.scale_numerator),
            "scale_denominator" => encode_f64(s.scale_denominator),
            "plot_type" => kind_tag(match s.plot_type {
                PlotType::LastScreenDisplay => "last-screen-display",
                PlotType::Extents => "extents", PlotType::Limits => "limits",
                PlotType::View => "view", PlotType::Window => "window",
                PlotType::Layout => "layout" }),
            "paper_units" => kind_tag(match s.paper_units {
                PlotPaperUnits::Millimeters => "millimeters",
                PlotPaperUnits::Inches => "inches", PlotPaperUnits::Pixels => "pixels" }),
            "provenance" => match &s.provenance {
                PlotProvenance::Imported => kind_tag("imported"),
                PlotProvenance::Authored => kind_tag("authored"),
                PlotProvenance::DefaultPage { reason } => row! {
                    "kind" => text("default-page"), "reason" => text(reason) },
            },
        }).collect()
    }
    fn encode_mleader_styles(db: &DrawingDatabase) -> Vec<Value> {
        db.mleader_styles.values().map(|s| row! {
            "id" => pid(&s.id), "name" => text(&s.name),
            "path_type" => kind_tag(match s.path_type {
                MLeaderPathType::Invisible => "invisible",
                MLeaderPathType::StraightLineSegments => "straight-line-segments",
                MLeaderPathType::Spline => "spline" }),
            "content_type" => Value::from(s.content_type),
            "text_height" => encode_f64(s.text_height),
            "arrowhead_size" => encode_f64(s.arrowhead_size),
            "landing_gap" => encode_f64(s.landing_gap),
            "dogleg_length" => encode_f64(s.dogleg_length),
            "text_style_name" => text(&s.text_style_name),
            "left_attachment" => Value::from(s.left_attachment),
            "text_angle_type" => kind_tag(match s.text_angle_type {
                MLeaderTextAngle::ParallelToLastLeaderLine => "parallel-to-last-leader-line",
                MLeaderTextAngle::Horizontal => "horizontal",
                MLeaderTextAngle::Optimized => "optimized" }),
        }).collect()
    }
    fn encode_linetypes(db: &DrawingDatabase) -> Vec<Value> {
        // `LinetypePattern` is flattened into the row, its field names kept.
        db.linetypes.values().map(|l| row! {
            "id" => pid(&l.id), "name" => text(&l.name),
            "elements" => f64s(&l.pattern.elements),
            "cycle" => encode_f64(l.pattern.cycle), "complex" => Value::Bool(l.complex),
        }).collect()
    }
    fn encode_xrefs(db: &DrawingDatabase) -> Vec<Value> {
        db.external_references.values().map(|r| row! {
            "name" => text(&r.name), "raw_path" => text(&r.raw_path),
            "resolved_path" => r.resolved_path.as_deref().map_or(Value::Null, Value::from),
            "block" => r.block.map_or(Value::Null, |block| pid(&block)),
            "overlay" => Value::Bool(r.overlay), "depth" => Value::from(r.depth),
            "status" => kind_tag(r.status.as_str()),
            "reason" => r.reason.as_deref().map_or(Value::Null, Value::from),
        }).collect()
    }
    /// Generate both directions of a flat row table: each field is a scalar
    /// with one encode helper and one [`Reader`] accessor.
    macro_rules! flat_table {
        ($encode_fn:ident, $decode_fn:ident, $db_map:ident, $ty:ident, $path:literal,
         { $($field:ident : $enc:ident / $take:ident),* $(,)? }) => {
            fn $encode_fn(db: &DrawingDatabase) -> Vec<Value> {
                db.$db_map.values().map(|row| {
                    let mut map = Map::new();
                    $( map.insert(stringify!($field).to_string(), $enc(&row.$field)); )*
                    Value::Object(map)
                }).collect()
            }
            fn $decode_fn(value: &Value) -> Result<$ty, String> {
                let mut r = Reader::new(value, $path)?;
                let decoded = $ty { $( $field: r.$take(stringify!($field))?, )* };
                r.finish()?; Ok(decoded)
            }
        };
    }
    flat_table!(encode_styles, decode_style, styles, Style, "style", {
        id: pid / id, name: text / string, resource_keys: strings / strings,
    });
    flat_table!(encode_scales, decode_scale, scales, Scale, "scale", {
        id: pid / id, name: text / string, paper_units: num / f64, drawing_units: num / f64,
    });
    flat_table!(encode_dim_styles, decode_dim_style, dim_styles, DimStyle, "dimension style", {
        id: pid / id, name: text / string, text_height: num / f64, arrow_size: num / f64,
        ext_offset: num / f64, ext_beyond: num / f64, text_gap: num / f64,
        text_vertical: int / i16, text_vertical_position: num / f64, scale: num / f64,
        decimals: int / i16, zero_suppression: int / i16, linear_factor: num / f64,
        rounding: num / f64, postfix: text / string, center_mark: num / f64,
        text_style_name: text / string, suppress_ext1: flag / boolean,
        suppress_ext2: flag / boolean, suppress_dim1: flag / boolean,
        suppress_dim2: flag / boolean, linear_unit_format: int / i16,
        decimal_separator: int / i16, text_justification: int / i16,
        text_inside_horizontal: flag / boolean, text_outside_horizontal: flag / boolean,
        text_fit: int / i16, text_inside_extensions: flag / boolean,
        text_outside_flow_line: flag / boolean, dim_line_color: int / i16,
        ext_line_color: int / i16, text_color: int / i16,
        dim_line_extension: num / f64, dim_line_increment: num / f64,
        continue_mode: flag / boolean,
    });
    /// Decode every row of one table keyed by a field (`value.<key>`).
    macro_rules! collect_by_field {
        ($tables:expr, $kind:ident, $row_fn:ident, $key:ident, $what:literal) => {{
            let mut map = BTreeMap::new();
            for row in rows($tables, TableKind::$kind) {
                let value = $row_fn(row).map_err(|d| row_error(TableKind::$kind, row, d))?;
                if map.insert(value.$key, value).is_some() {
                    return Err(row_error(TableKind::$kind, row,
                                         format!("duplicate {} row", $what)));
                }
            }
            map
        }};
    }
    pub(crate) fn decode(tables: &TableSnapshot) -> Result<DecodedTables, SnapshotError> {
        let (layers, layer_attributes) = decode_layers(tables)?;
        let b = decode_blocks(tables)?;
        Ok(DecodedTables {
            layers, layer_attributes, blocks: b.blocks, block_metadata: b.metadata,
            block_name_order: b.name_order, block_source_handles: b.handles,
            layouts: collect_by_field!(tables, Layouts, decode_layout_row, id, "layout"),
            plot_settings: collect_by_field!(tables, PlotSettings, decode_plot_row, layout,
                                             "plot settings"),
            styles: collect_by_field!(tables, Styles, decode_style, id, "style"),
            dim_styles: collect_by_field!(tables, DimStyles, decode_dim_style, id,
                                          "dimension style"),
            mleader_styles: collect_by_field!(tables, MLeaderStyles, decode_mleader_style, id,
                                              "mleader style"),
            linetypes: collect_by_field!(tables, Linetypes, decode_linetype, id, "linetype"),
            scales: collect_by_field!(tables, Scales, decode_scale, id, "scale"),
            external_references: decode_xrefs(tables)?,
        })
    }
    /// The two maps one layer row feeds.
    type LayerTables = (BTreeMap<LayerId, Layer>, BTreeMap<LayerId, LayerAttributes>);
    fn decode_layers(tables: &TableSnapshot) -> Result<LayerTables, SnapshotError> {
        let mut layers = BTreeMap::new();
        let mut attributes = BTreeMap::new();
        for row in rows(tables, TableKind::Layers) {
            let (layer, attrs) = decode_layer_row(row)
                .map_err(|detail| row_error(TableKind::Layers, row, detail))?;
            let id = layer.id;
            if layers.insert(id, layer).is_some() {
                return Err(row_error(TableKind::Layers, row, "duplicate layer row id"));
            }
            attributes.insert(id, attrs);
        }
        Ok((layers, attributes))
    }
    fn decode_layer_row(row: &Value) -> Result<(Layer, LayerAttributes), String> {
        let mut reader = Reader::new(row, "layer")?;
        let id: LayerId = reader.id("id")?;
        let name = reader.string("name")?;
        let attributes = decode_layer_attributes(&reader.take("attributes")?)?;
        reader.finish()?;
        // `visible` is derived, never persisted: the pred. `layer_edit` uses.
        Ok((Layer { id, name, visible: attributes.is_visible() }, attributes))
    }
    fn decode_layer_attributes(value: &Value) -> Result<LayerAttributes, String> {
        let mut reader = Reader::new(value, "attributes")?;
        let color = decode_layer_color(&reader.take("color")?)?;
        let lineweight = decode_line_weight(&reader.take("lineweight")?)?;
        let attributes = LayerAttributes {
            off: reader.boolean("off")?, frozen: reader.boolean("frozen")?,
            frozen_in_new_viewport: reader.boolean("frozen_in_new_viewport")?,
            locked: reader.boolean("locked")?,
            xref_dependent: reader.boolean("xref_dependent")?,
            plottable: reader.boolean("plottable")?, color,
            linetype: reader.string("linetype")?, lineweight, opacity: reader.f32("opacity")?,
            plot_style: reader.opt_string("plot_style")? };
        reader.finish()?;
        Ok(attributes)
    }
    fn decode_layer_color(value: &Value) -> Result<LayerColor, String> {
        let mut reader = Reader::new(value, "color")?;
        let aci = match reader.take("aci")? {
            Value::Null => None,
            other => Some(value_of_uint(&other, "color.aci")?),
        };
        let rgb_value = reader.take("rgb")?;
        let rgb_items = rgb_value.as_array().filter(|i| i.len() == 3)
            .ok_or_else(|| "color.rgb: expected three bytes".to_string())?;
        let rgb = [value_of_uint(&rgb_items[0], "color.rgb[0]")?,
                   value_of_uint(&rgb_items[1], "color.rgb[1]")?,
                   value_of_uint(&rgb_items[2], "color.rgb[2]")?];
        reader.finish()?; Ok(LayerColor { aci, rgb })
    }
    fn decode_line_weight(value: &Value) -> Result<LayerLineWeight, String> {
        let mut reader = Reader::new(value, "lineweight")?;
        let weight = match reader.string("kind")?.as_str() {
            "hundredths" => {
                let raw = reader.take("value")?;
                LayerLineWeight::Hundredths(value_of_uint(&raw, "lineweight.value")?) }
            "default" => LayerLineWeight::Default,
            "by-layer" => LayerLineWeight::ByLayer,
            other => return Err(format!("lineweight: unknown kind tag {other:?}")),
        };
        reader.finish()?;
        Ok(weight)
    }
    struct DecodedBlocks {
        blocks: BTreeMap<BlockId, BlockDefinition>,
        metadata: BTreeMap<BlockId, BlockMetadata>,
        handles: BTreeMap<u64, BlockId>,
        name_order: Vec<(String, BlockId)>,
    }
    struct BlockRow {
        definition: BlockDefinition,
        metadata: Option<BlockMetadata>,
        source_handle: Option<u64>,
        registration_index: u32,
    }
    fn decode_blocks(tables: &TableSnapshot) -> Result<DecodedBlocks, SnapshotError> {
        let mut block_rows = Vec::new();
        for row in rows(tables, TableKind::Blocks) {
            block_rows.push(decode_block_row(row)
                .map_err(|detail| row_error(TableKind::Blocks, row, detail))?);
        }
        // Sorting on `registration_index` rebuilds `block_name_order` (see
        // `encode_blocks`). Duplicate indices would make the order ambiguous.
        block_rows.sort_by_key(|row| row.registration_index);
        let mut decoded = DecodedBlocks {
            blocks: BTreeMap::new(), metadata: BTreeMap::new(),
            handles: BTreeMap::new(), name_order: Vec::new() };
        let mut previous: Option<u32> = None;
        let duplicate = |what: String| SnapshotError::invalid(format!(
            "{}: duplicate {what}", table_entry(TableKind::Blocks)));
        for row in block_rows {
            let id = row.definition.id;
            if previous == Some(row.registration_index) {
                return Err(duplicate(format!("registration_index {}", row.registration_index)));
            }
            previous = Some(row.registration_index);
            if let Some(handle) = row.source_handle {
                if decoded.handles.insert(handle, id).is_some() {
                    return Err(duplicate(format!("source_handle {handle}")));
                }
            }
            if let Some(metadata) = row.metadata {
                decoded.name_order.push((metadata.name.clone(), id));
                decoded.metadata.insert(id, metadata);
            }
            if decoded.blocks.insert(id, row.definition).is_some() {
                return Err(duplicate(format!("block row id {}", id.0)))
            }
        }
        Ok(decoded)
    }
    fn decode_block_row(row: &Value) -> Result<BlockRow, String> {
        let mut reader = Reader::new(row, "block")?;
        let id: BlockId = reader.id("id")?;
        let (name, base_point) = (reader.opt_string("name")?, reader.opt_point3("base_point")?);
        let entities = reader.array("entities", |v| value_of_id(v, "entities[]"))?;
        let dynamic_visibility = match reader.take("dynamic_visibility")? {
            Value::Null => None, value => Some(decode_dynamic_visibility(&value)?) };
        let source_handle = reader.opt_u64("source_handle")?;
        let registration_index = reader.u32("registration_index")?;
        reader.finish()?;
        // The builder records name and base point together or not at all; half
        // a pair is a malformed row, not an unnamed block with an origin.
        let metadata = match (name, base_point) {
            (Some(name), Some(base_point)) => Some(BlockMetadata { name, base_point }),
            (None, None) => None,
            _ => return Err("`name` and `base_point` must be both present or both null".into()),
        };
        Ok(BlockRow { definition: BlockDefinition { id, entities, dynamic_visibility },
                      metadata, source_handle, registration_index })
    }
    fn decode_dynamic_visibility(value: &Value) -> Result<DynamicBlockVisibility, String> {
        let mut reader = Reader::new(value, "dynamic_visibility")?;
        let member_entities = reader.array("member_entities",
            |value| value_of_id(value, "member_entities[]"))?;
        let states = reader.array("states", decode_dynamic_state)?;
        let active_state = reader.opt_string("active_state")?;
        reader.finish()?;
        Ok(DynamicBlockVisibility { member_entities, states, active_state })
    }
    fn decode_dynamic_state(value: &Value) -> Result<DynamicBlockState, String> {
        let mut reader = Reader::new(value, "dynamic_visibility.state")?;
        let name = reader.string("name")?;
        let entities = reader.array("entities",
            |value| value_of_id(value, "states[].entities[]"))?;
        reader.finish()?;
        Ok(DynamicBlockState { name, entities })
    }
    fn decode_layout_row(row: &Value) -> Result<Layout, String> {
        let mut reader = Reader::new(row, "layout")?;
        let id: LayoutId = reader.id("id")?;
        if id.0 == 0 {
            return Err("layout id 0 is synthetic model space and has no row".into());
        }
        let name = reader.string("name")?;
        let viewports = reader.array("viewports", decode_viewport)?;
        reader.finish()?;
        Ok(Layout { id, name, viewports })
    }
    fn decode_viewport(value: &Value) -> Result<PaperViewport, String> {
        let mut reader = Reader::new(value, "viewport")?;
        let clip_value = reader.take("clip")?;
        let clip_items = clip_value.as_array()
            .ok_or_else(|| "clip: expected an array".to_string())?;
        let mut clip = Vec::with_capacity(clip_items.len());
        for item in clip_items {
            clip.push(decode_point3(item).map_err(|e| format!("clip[]: {e}"))?)
        }
        let model_to_paper = decode_transform3(&reader.take("model_to_paper")?)?;
        let completeness = decode_completeness(&reader.take("completeness")?)?;
        let overrides = reader.take("layer_overrides")?;
        let override_rows = overrides.as_array()
            .ok_or_else(|| "layer_overrides: expected an array".to_string())?;
        let mut layer_overrides = BTreeMap::new();
        for override_row in override_rows {
            let mut entry = Reader::new(override_row, "layer_override")?;
            let layer: LayerId = entry.id("layer")?;
            let frozen = entry.boolean("frozen")?;
            entry.finish()?;
            if layer_overrides.insert(layer, frozen).is_some() {
                return Err(format!("layer_overrides: duplicate layer {}", layer.0));
            }
        }
        reader.finish()?;
        Ok(PaperViewport { clip, model_to_paper, completeness, layer_overrides })
    }
    fn decode_transform3(value: &Value) -> Result<Transform3, String> {
        let lines = value.as_array().filter(|lines| lines.len() == 4)
            .ok_or_else(|| "model_to_paper: expected four rows of four numbers".to_string())?;
        let mut matrix = [[0.0f64; 4]; 4];
        for (row_index, line) in lines.iter().enumerate() {
            let cells = line.as_array().filter(|cells| cells.len() == 4)
                .ok_or_else(|| format!("model_to_paper[{row_index}]: expected four numbers"))?;
            for (cell_index, cell) in cells.iter().enumerate() {
                matrix[row_index][cell_index] = value_of_f64(cell, "model_to_paper[]")?;
            }
        }
        Ok(Transform3 { matrix })
    }
    fn decode_completeness(value: &Value) -> Result<Completeness, String> {
        let mut reader = Reader::new(value, "completeness")?;
        let completeness = match reader.string("kind")?.as_str() {
            "complete" => Completeness::Complete,
            "partial" => Completeness::Partial(reader.strings("reasons")?),
            "missing" => Completeness::Missing(reader.strings("reasons")?),
            "unverified" => Completeness::Unverified,
            other => return Err(format!("completeness: unknown kind tag {other:?}")),
        };
        reader.finish()?; Ok(completeness)
    }
    fn decode_plot_row(row: &Value) -> Result<PlotSettingsRecord, String> {
        let mut reader = Reader::new(row, "plot settings")?;
        let layout: LayoutId = reader.id("layout")?;
        let paper_size_name = reader.string("paper_size_name")?;
        let (paper_width, paper_height) =
            (reader.f64("paper_width")?, reader.f64("paper_height")?);
        let margins_value = reader.take("margins")?;
        let mut margins_reader = Reader::new(&margins_value, "margins")?;
        let margins = PlotMargins { left: margins_reader.f64("left")?,
            bottom: margins_reader.f64("bottom")?, right: margins_reader.f64("right")?,
            top: margins_reader.f64("top")? };
        margins_reader.finish()?;
        let rotation = read_unit_tag(&reader.take("rotation")?, "rotation", &[
            ("none", PlotRotation::None), ("degrees-90", PlotRotation::Degrees90),
            ("degrees-180", PlotRotation::Degrees180),
            ("degrees-270", PlotRotation::Degrees270) ])?;
        let scale_numerator = reader.f64("scale_numerator")?;
        let scale_denominator = reader.f64("scale_denominator")?;
        let plot_type = read_unit_tag(&reader.take("plot_type")?, "plot_type", &[
            ("last-screen-display", PlotType::LastScreenDisplay), ("extents", PlotType::Extents),
            ("limits", PlotType::Limits), ("view", PlotType::View),
            ("window", PlotType::Window), ("layout", PlotType::Layout) ])?;
        let paper_units = read_unit_tag(&reader.take("paper_units")?, "paper_units", &[
            ("millimeters", PlotPaperUnits::Millimeters), ("inches", PlotPaperUnits::Inches),
            ("pixels", PlotPaperUnits::Pixels) ])?;
        let provenance_value = reader.take("provenance")?;
        let mut provenance_reader = Reader::new(&provenance_value, "provenance")?;
        let provenance = match provenance_reader.string("kind")?.as_str() {
            "imported" => PlotProvenance::Imported,
            "authored" => PlotProvenance::Authored,
            "default-page" => PlotProvenance::DefaultPage {
                reason: provenance_reader.string("reason")? },
            other => return Err(format!("provenance: unknown kind tag {other:?}")),
        };
        provenance_reader.finish()?; reader.finish()?;
        Ok(PlotSettingsRecord { layout, paper_size_name, paper_width, paper_height, margins,
            rotation, scale_numerator, scale_denominator, plot_type, paper_units, provenance })
    }
    fn decode_mleader_style(row: &Value) -> Result<MLeaderStyle, String> {
        let mut reader = Reader::new(row, "multileader style")?;
        let style = MLeaderStyle { id: reader.id("id")?, name: reader.string("name")?,
            path_type: read_unit_tag(&reader.take("path_type")?, "path_type", &[
                ("invisible", MLeaderPathType::Invisible),
                ("straight-line-segments", MLeaderPathType::StraightLineSegments),
                ("spline", MLeaderPathType::Spline) ])?,
            content_type: reader.i16("content_type")?,
            text_height: reader.f64("text_height")?,
            arrowhead_size: reader.f64("arrowhead_size")?,
            landing_gap: reader.f64("landing_gap")?,
            dogleg_length: reader.f64("dogleg_length")?,
            text_style_name: reader.string("text_style_name")?,
            left_attachment: reader.i16("left_attachment")?,
            text_angle_type: read_unit_tag(&reader.take("text_angle_type")?,
                "text_angle_type", &[
                    ("parallel-to-last-leader-line",
                     MLeaderTextAngle::ParallelToLastLeaderLine),
                    ("horizontal", MLeaderTextAngle::Horizontal),
                    ("optimized", MLeaderTextAngle::Optimized) ])?,
        };
        reader.finish()?;
        Ok(style)
    }
    fn decode_linetype(row: &Value) -> Result<LineType, String> {
        let mut reader = Reader::new(row, "linetype")?;
        let linetype = LineType {
            id: reader.id("id")?, name: reader.string("name")?,
            pattern: LinetypePattern {
                elements: reader.array("elements",
                    |value| value_of_f64(value, "elements[]"))?,
                cycle: reader.f64("cycle")? },
            complex: reader.boolean("complex")?,
        };
        reader.finish()?; Ok(linetype)
    }
    fn decode_xrefs(
        tables: &TableSnapshot,
    ) -> Result<BTreeMap<String, ExternalReference>, SnapshotError> {
        let mut references = BTreeMap::new();
        for row in rows(tables, TableKind::ExternalReferences) {
            let reference = decode_xref(row)
                .map_err(|detail| row_error(TableKind::ExternalReferences, row, detail))?;
            let name = reference.name.clone();
            if references.insert(name, reference).is_some() {
                return Err(row_error(TableKind::ExternalReferences, row,
                                     "duplicate external reference name"))
            }
        }
        Ok(references)
    }
    fn decode_xref(row: &Value) -> Result<ExternalReference, String> {
        let mut reader = Reader::new(row, "external reference")?;
        let reference = ExternalReference {
            name: reader.string("name")?, raw_path: reader.string("raw_path")?,
            resolved_path: reader.opt_string("resolved_path")?, block: reader.opt_id("block")?,
            overlay: reader.boolean("overlay")?, depth: reader.u32("depth")?,
            status: read_unit_tag(&reader.take("status")?, "status", &[
                ("loaded", ExternalReferenceStatus::Loaded),
                ("missing", ExternalReferenceStatus::Missing),
                ("browser-unavailable", ExternalReferenceStatus::BrowserUnavailable),
                ("refused-path", ExternalReferenceStatus::RefusedPath),
                ("unloaded", ExternalReferenceStatus::Unloaded),
                ("cycle", ExternalReferenceStatus::Cycle),
                ("depth-exceeded", ExternalReferenceStatus::DepthExceeded),
                ("skipped-overlay", ExternalReferenceStatus::SkippedOverlay),
                ("unsupported", ExternalReferenceStatus::Unsupported) ])?,
            reason: reader.opt_string("reason")?,
        };
        reader.finish()?;
        Ok(reference)
    }
    /// The rows of one table; an absent table is an empty slice.
    fn rows(tables: &TableSnapshot, kind: TableKind) -> &[Value] {
        tables.get(kind).unwrap_or(&[])
    }
    /// Name the table and, when the row carries one, its id.
    fn row_error(kind: TableKind, row: &Value, detail: impl std::fmt::Display) -> SnapshotError {
        let id = row.get("id").and_then(Value::as_i64)
            .or_else(|| row.get("layout").and_then(Value::as_i64));
        match id {
            Some(id) => SnapshotError::invalid(
                format!("{} row id {id}: {detail}", table_entry(kind))),
            None => SnapshotError::invalid(format!("{}: {detail}", table_entry(kind))),
        }
    }
    /// A persisted identifier as a JSON number (`(kind << 45) | seq`).
    fn pid<T: PersistentIdOf>(id: &T) -> Value { Value::from(id.persistent().raw()) }
    fn entity_ids(ids: &[EntityId]) -> Value {
        Value::Array(ids.iter().map(pid).collect()) }
    fn f64s(values: &[f64]) -> Value {
        Value::Array(values.iter().map(|v| encode_f64(*v)).collect()) }
    fn bytes(values: &[u8; 3]) -> Value {
        Value::Array(values.iter().map(|v| Value::from(*v)).collect()) }
    fn text(value: &str) -> Value { Value::from(value) }
    fn num(value: &f64) -> Value { encode_f64(*value) }
    fn int(value: &i16) -> Value { Value::from(*value) }
    fn flag(value: &bool) -> Value { Value::Bool(*value) }
    /// A unit-only enum variant: `{"kind": "<tag>"}`.
    fn kind_tag(tag: &str) -> Value { row! { "kind" => text(tag) } }
    fn strings(values: &[String]) -> Value {
        Value::Array(values.iter().map(|v| text(v)).collect()) }
    /// Read a `{"kind": "<tag>"}` object with no other field.
    fn read_unit_tag<T: Copy>(
        value: &Value, field: &str, variants: &[(&str, T)],
    ) -> Result<T, String> {
        let mut reader = Reader::new(value, "tag")?;
        let tag = reader.string("kind")?;
        let matched = variants.iter().find(|(name, _)| *name == tag)
            .map(|(_, variant)| *variant)
            .ok_or_else(|| format!("{field}: unknown kind tag {tag:?}"))?;
        reader.finish()?;
        Ok(matched)
    }
    /// A strict reader for one JSON object. Fields are taken by name; a required
    /// one that is absent is an error, and [`Reader::finish`] refuses leftovers.
    /// That is ADR-0037 D11's rule for rows: this writer re-serializes whole
    /// rows, so a field it does not model would be dropped by the next save.
    struct Reader {
        map: Map<String, Value>,
        context: &'static str,
    }
    impl Reader {
        fn new(value: &Value, context: &'static str) -> Result<Self, String> {
            match value { Value::Object(map) => Ok(Reader { map: map.clone(), context }),
                          _ => Err(format!("{context}: expected a JSON object")) }
        }
        fn take(&mut self, field: &str) -> Result<Value, String> {
            self.map.remove(field).ok_or_else(|| format!(
                "{}: required field `{field}` is missing", self.context)) }
        fn finish(self) -> Result<(), String> {
            match self.map.keys().next() {
                Some(key) => Err(format!("{}: unknown field `{key}`", self.context)),
                None => Ok(()) }
        }
        fn string(&mut self, field: &str) -> Result<String, String> {
            let v = self.take(field)?; value_of_str(&v, field) }
        fn i16(&mut self, field: &str) -> Result<i16, String> {
            let v = self.take(field)?; value_of_i16(&v, field) }
        fn u32(&mut self, field: &str) -> Result<u32, String> {
            let v = self.take(field)?; value_of_u32(&v, field) }
        fn f64(&mut self, field: &str) -> Result<f64, String> {
            let v = self.take(field)?; value_of_f64(&v, field) }
        fn f32(&mut self, field: &str) -> Result<f32, String> {
            Ok(self.f64(field)? as f32) }
        fn boolean(&mut self, field: &str) -> Result<bool, String> {
            let v = self.take(field)?;
            v.as_bool().ok_or_else(|| format!("{field}: expected a boolean")) }
        fn opt_string(&mut self, field: &str) -> Result<Option<String>, String> {
            match self.take(field)? { Value::Null => Ok(None),
                v => Ok(Some(value_of_str(&v, field)?)) }
        }
        fn opt_u64(&mut self, field: &str) -> Result<Option<u64>, String> {
            match self.take(field)? { Value::Null => Ok(None),
                v => Ok(Some(value_of_uint(&v, field)?)) }
        }
        fn opt_point3(&mut self, field: &str) -> Result<Option<Point3>, String> {
            match self.take(field)? { Value::Null => Ok(None),
                value => decode_point3(&value).map(Some)
                    .map_err(|error| format!("{field}: {error}")) }
        }
        fn id<T: PersistentIdOf>(&mut self, field: &str) -> Result<T, String> {
            let v = self.take(field)?; value_of_id(&v, field) }
        fn opt_id<T: PersistentIdOf>(&mut self, field: &str) -> Result<Option<T>, String> {
            match self.take(field)? { Value::Null => Ok(None),
                v => Ok(Some(value_of_id(&v, field)?)) } }
        fn strings(&mut self, field: &str) -> Result<Vec<String>, String> {
            self.array(field, |value| value_of_str(value, field)) }
        fn array<T>(&mut self, field: &str,
                    each: impl Fn(&Value) -> Result<T, String>) -> Result<Vec<T>, String> {
            let value = self.take(field)?;
            let items = value.as_array()
                .ok_or_else(|| format!("{field}: expected an array"))?;
            items.iter().map(each).collect()
        }
    }
    fn value_of_str(value: &Value, field: &str) -> Result<String, String> {
        value.as_str().map(str::to_string)
            .ok_or_else(|| format!("{field}: expected a string"))
    }
    fn value_of_i16(value: &Value, field: &str) -> Result<i16, String> {
        let raw = value.as_i64().ok_or_else(|| format!("{field}: expected an integer"))?;
        i16::try_from(raw).map_err(|_| format!("{field}: {raw} is out of range"))
    }
    fn value_of_u32(value: &Value, field: &str) -> Result<u32, String> {
        value_of_uint(value, field) }
    /// Any unsigned scalar small enough for its target type.
    fn value_of_uint<T: TryFrom<u64>>(value: &Value, field: &str) -> Result<T, String> {
        let raw = value.as_u64()
            .ok_or_else(|| format!("{field}: expected an unsigned integer"))?;
        T::try_from(raw).map_err(|_| format!("{field}: {raw} is out of range"))
    }
    fn value_of_f64(value: &Value, field: &str) -> Result<f64, String> {
        decode_f64(value).map_err(|error| format!("{field}: {error}"))
    }
    /// Decode a persistent id and check it belongs to `T`'s family. A bare
    /// sequence cannot be built by [`PersistentId::from_raw`], and a kind
    /// mismatch would alias another table's row.
    fn value_of_id<T: PersistentIdOf>(value: &Value, field: &str) -> Result<T, String> {
        let raw = value.as_i64()
            .ok_or_else(|| format!("{field}: expected a persistent id (a JSON integer)"))?;
        let id = PersistentId::from_raw(raw)
            .ok_or_else(|| format!("{field}: {raw} is not a persistent id"))?;
        if id.kind() != T::KIND {
            return Err(format!("{field}: id {raw} is a {:?} id, expected a {:?} id",
                               id.kind(), T::KIND));
        }
        Ok(T::from_seq(id.seq()))
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::entity::{DbEntity, DbObject};
        use crate::DrawingDatabaseBuilder;
        use cad_domain::ymodel::{IdKind, SEQ_BITS};
        use cad_domain::{DatabaseId, ObjectId, Revision, SemanticGeometry, SpaceId};

        fn point(x: f64, y: f64) -> Point3 { Point3 { x, y, z: 0.0 } }

        fn fixture() -> DrawingDatabase {
            let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
            b.insert_layer(Layer { id: LayerId(1), name: "Walls".into(), visible: false })
                .unwrap();
            b.set_layer_attributes(LayerId(1), LayerAttributes {
                off: true, frozen: true,
                color: LayerColor { aci: Some(1), rgb: [255, 0, 0] },
                linetype: "DASHED".into(), lineweight: LayerLineWeight::Hundredths(35),
                opacity: 0.5, ..LayerAttributes::default() }).unwrap();
            b.insert_entity(DbEntity {
                object: DbObject { id: ObjectId(1), type_key: "AcDbLine".into(),
                                   revision: Revision(0), source_handle: None },
                id: EntityId(1), layer: LayerId(1), space: SpaceId::Block(BlockId(1)),
                geometry: SemanticGeometry::Line {
                    start: point(0.0, 0.0), end: point(10.0, 0.0) },
                draw_order: 0 }).unwrap();
            for id in [BlockId(1), BlockId(2)] {
                b.insert_block(BlockDefinition {
                    id, entities: vec![EntityId(1)], dynamic_visibility: None }).unwrap();
            }
            // Registered out of id order: last-duplicate-wins depends on it.
            b.set_block_metadata(BlockId(2), "Window", point(0.0, 0.0)).unwrap();
            b.set_block_metadata(BlockId(1), "Door", point(1.5, 2.5)).unwrap();
            b.set_block_source_handle(BlockId(1), 42);
            b.insert_layout(Layout {
                id: LayoutId(0), name: "Model".into(), viewports: vec![] }).unwrap();
            let viewport = |overrides: BTreeMap<LayerId, bool>| PaperViewport {
                clip: vec![point(0.0, 0.0), point(100.0, 0.0),
                           point(100.0, 50.0), point(0.0, 50.0)],
                model_to_paper: Transform3::translation(point(5.0, 7.0)),
                completeness: Completeness::Partial(vec!["viewport.clip_partial".into()]),
                layer_overrides: overrides };
            b.insert_layout(Layout { id: LayoutId(1), name: "Sheet".into(),
                viewports: vec![viewport(BTreeMap::new()),
                                viewport(BTreeMap::from([(LayerId(1), true)]))] }).unwrap();
            b.set_plot_settings(PlotSettingsRecord {
                layout: LayoutId(1), paper_size_name: "ISO_A3".into(),
                paper_width: 297.0, paper_height: 420.0,
                margins: PlotMargins::uniform(12.0), rotation: PlotRotation::Degrees90,
                scale_numerator: 1.0, scale_denominator: 100.0,
                plot_type: PlotType::Layout, paper_units: PlotPaperUnits::Millimeters,
                provenance: PlotProvenance::DefaultPage {
                    reason: "plot.default_page".into() } }).unwrap();
            b.insert_style(Style { id: StyleId(1), name: "Notes".into(),
                resource_keys: vec!["arial.ttf".into()] }).unwrap();
            let mut dim = DimStyle::standard_metric(DimStyleId(3));
            dim.name = "Fancy".into(); dim.text_height = 3.5;
            b.insert_dim_style(dim).unwrap();
            let mut mleader = MLeaderStyle::standard_metric(MLeaderStyleId(4));
            mleader.name = "Callout".into(); mleader.left_attachment = 3;
            b.insert_mleader_style(mleader).unwrap();
            b.insert_linetype(LineType { id: LinetypeId(1), name: "DASHED".into(),
                pattern: LinetypePattern::from_elements([0.5, -0.25]), complex: false })
                .unwrap();
            b.insert_scale(Scale { id: ScaleId(1), name: "1:100".into(),
                paper_units: 1.0, drawing_units: 100.0 }).unwrap();
            b.insert_external_reference(ExternalReference {
                name: "Site".into(), raw_path: "site.dwg".into(),
                resolved_path: Some("site.dwg".into()), block: Some(BlockId(2)),
                overlay: false, depth: 0, status: ExternalReferenceStatus::Missing,
                reason: Some("xref.missing".into()) }).unwrap();
            b.finish().unwrap()
        }

        #[test]
        fn round_trip_preserves_every_table_row() {
            let db = fixture();
            let decoded = decode(&encode(&db).unwrap()).unwrap();
            assert_eq!(decoded.layers, db.layers);
            assert_eq!(decoded.layer_attributes, db.layer_attributes);
            assert_eq!(decoded.blocks, db.blocks);
            assert_eq!(decoded.block_metadata, db.block_metadata);
            assert_eq!(decoded.block_source_handles, db.block_source_handles);
            assert_eq!(decoded.block_name_order,
                       vec![("Window".into(), BlockId(2)), ("Door".into(), BlockId(1))]);
            let mut layouts = db.layouts.clone();
            layouts.remove(&LayoutId(0));
            assert_eq!(decoded.layouts, layouts, "id 0 is synthetic model space");
            assert_eq!(decoded.plot_settings, db.plot_settings);
            assert_eq!(decoded.styles, db.styles);
            assert_eq!(decoded.dim_styles, db.dim_styles);
            assert_eq!(decoded.mleader_styles, db.mleader_styles);
            assert_eq!(decoded.linetypes, db.linetypes);
            assert_eq!(decoded.scales, db.scales);
            assert_eq!(decoded.external_references, db.external_references);
        }

        #[test]
        fn non_finite_f64_survives_bit_exactly() {
            let mut b = DrawingDatabaseBuilder::new(DatabaseId(9));
            b.insert_layout(Layout {
                id: LayoutId(1), name: "S".into(), viewports: vec![] }).unwrap();
            b.set_plot_settings(PlotSettingsRecord {
                layout: LayoutId(1), paper_size_name: "CUSTOM".into(),
                paper_width: f64::NAN, paper_height: 1.0,
                margins: PlotMargins { left: f64::NEG_INFINITY, ..PlotMargins::default() },
                rotation: PlotRotation::None, scale_numerator: 1.0,
                scale_denominator: f64::NAN, plot_type: PlotType::Window,
                paper_units: PlotPaperUnits::Millimeters,
                provenance: PlotProvenance::Authored }).unwrap();
            let tables = encode(&b.finish().unwrap()).unwrap();
            let text = serde_json::to_string(
                tables.get(TableKind::PlotSettings).unwrap()).unwrap();
            assert!(text.contains("$f64") && !text.contains("null"), "{text}");
            let decoded = decode(&tables).unwrap();
            let plot = &decoded.plot_settings[&LayoutId(1)];
            assert_eq!(plot.paper_width.to_bits(), f64::NAN.to_bits());
            assert_eq!(plot.margins.left.to_bits(), f64::NEG_INFINITY.to_bits());
            assert_eq!(plot.scale_denominator.to_bits(), f64::NAN.to_bits());
        }

        #[test]
        fn malformed_rows_are_refused_with_table_and_id() {
            let mutate = |kind: TableKind, edit: &dyn Fn(&mut Map<String, Value>)| {
                let mut tables = encode(&fixture()).unwrap();
                let mut rows = tables.get(kind).expect("the table is written").to_vec();
                edit(rows[0].as_object_mut().expect("a row object"));
                tables.set(kind, rows);
                decode(&tables).expect_err("the row must be refused")
            };
            let extra = mutate(TableKind::Layers,
                &|row| { row.insert("visible".into(), Value::Bool(true)); });
            assert_eq!(extra.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
            assert!(extra.detail.contains("tables/layers.json"), "{}", extra.detail);
            assert!(extra.detail.contains("unknown field"), "{}", extra.detail);
            let missing = mutate(TableKind::Styles, &|row| { row.remove("id"); });
            assert!(missing.detail.contains("tables/styles.json"), "{}", missing.detail);
            assert!(missing.detail.contains("required field `id`"), "{}", missing.detail);
            let wrong = mutate(TableKind::Layers, &|row| {
                row.insert("id".into(), Value::from(EntityId(1).persistent().raw())); });
            assert!(wrong.detail.contains("expected a Layer id"), "{}", wrong.detail);
            let tag = mutate(TableKind::PlotSettings, &|row| {
                row.insert("rotation".into(), serde_json::json!({ "kind": "sideways" })); });
            assert!(tag.detail.contains("rotation") && tag.detail.contains("sideways"),
                    "{}", tag.detail);
        }

        fn assert_id(row: &Value, field: &str, kind: IdKind) {
            let raw = row[field].as_i64().expect("a JSON integer id");
            assert!(raw >= 1i64 << SEQ_BITS, "{field} is a bare sequence: {raw}");
            assert_eq!(PersistentId::from_raw(raw).expect("a persistent id").kind(), kind);
        }

        #[test]
        fn every_encoded_identifier_is_a_persistent_composite() {
            let tables = encode(&fixture()).unwrap();
            let rows_of = |kind| tables.get(kind).expect("the table is written");
            rows_of(TableKind::Layers).iter().for_each(|r| assert_id(r, "id", IdKind::Layer));
            for row in rows_of(TableKind::Blocks) {
                assert_id(row, "id", IdKind::Block);
                for entity in row["entities"].as_array().expect("member ids") {
                    let raw = entity.as_i64().expect("a JSON integer id");
                    assert_eq!(PersistentId::from_raw(raw).expect("an id").kind(), IdKind::Entity);
                }
            }
            for row in rows_of(TableKind::Layouts) {
                assert_id(row, "id", IdKind::Layout);
                for viewport in row["viewports"].as_array().expect("viewports") {
                    let overrides = viewport["layer_overrides"].as_array().expect("overrides");
                    overrides.iter().for_each(|e| assert_id(e, "layer", IdKind::Layer));
                }
            }
            rows_of(TableKind::PlotSettings).iter()
                .for_each(|r| assert_id(r, "layout", IdKind::Layout));
            for (kind, family) in [(TableKind::Styles, IdKind::Style),
                                   (TableKind::DimStyles, IdKind::DimStyle),
                                   (TableKind::MLeaderStyles, IdKind::MLeaderStyle),
                                   (TableKind::Linetypes, IdKind::Linetype),
                                   (TableKind::Scales, IdKind::Scale)] {
                rows_of(kind).iter().for_each(|row| assert_id(row, "id", family));
            }
            for row in rows_of(TableKind::ExternalReferences) {
                if !row["block"].is_null() { assert_id(row, "block", IdKind::Block) }
            }
        }

        #[test]
        fn absent_tables_decode_to_empty_maps() {
            assert_eq!(decode(&TableSnapshot::default()).unwrap(), DecodedTables::default());
        }
    }
}

pub(crate) use codec::{decode, encode};
