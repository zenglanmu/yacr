//! Selection identity and read-only property extraction (spec F05).
//!
//! A selection is a list of [`SelectionRef`]s. Its identity is
//! `(entity, instance_path)`: two INSERT references to the same block entity at
//! different placements are **different** selected objects, and a sub-element
//! (for example one mesh face) is distinct again. Nothing here mutates the DWG;
//! a selection only chooses what to inspect on the CPU side.
//!
//! Property extraction is a pure read over [`DrawingDatabase`]: given a
//! `SelectionRef` it produces a small, ordered set of [`PropertyRow`]s
//! (id/handle/type/layer/space plus geometry-specific basics). Unknown or
//! unsupported geometry yields an explicit row rather than invented values.
//!
//! Multi-selection reports the **intersection** of the row keys: only
//! properties common to every selected object are listed, each carrying one
//! value when all refs agree or an explicit mixed marker otherwise. Keys that
//! exist on only some selected objects are not listed (no fabricated blanks).

use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::{
    EntityId, HatchFill, InstancePath, MLeaderContent, Point3, SelectionRef, SemanticGeometry,
    SpaceId,
};
use std::collections::BTreeSet;

/// Maximum number of `enum` options emitted for one property row.
///
/// A drawing can carry thousands of layers or linetypes; the panel is a picker,
/// not a table dump, so options are capped after sorting. The cap is a hard,
/// documented bound (not a silent truncation of the wire shape): a consumer
/// that needs the full table uses the layer/linetype queries instead.
pub const MAX_ENUM_OPTIONS: usize = 256;

/// The panel group a property row belongs to (stable machine key, never text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyGroup {
    /// Object identity and placement: id/handle/type/layer/space/draw order.
    Entity,
    /// Per-geometry values (start/end/length/radius/...).
    Geometry,
    /// Text-content and text-style values.
    Text,
    /// Display attributes and appearance flags.
    Display,
}

impl PropertyGroup {
    /// Stable machine key carried on the wire (`entity`, `geometry`, …).
    pub const fn key(self) -> &'static str {
        match self {
            PropertyGroup::Entity => "entity",
            PropertyGroup::Geometry => "geometry",
            PropertyGroup::Text => "text",
            PropertyGroup::Display => "display",
        }
    }

    /// The group for a property key.
    ///
    /// This is the single mapping for every key the module emits. `style` is
    /// text by default (MULTILEADER style names) and is overridden to
    /// [`PropertyGroup::Display`] for the hatch style row. `color`,
    /// `linetype`, `lineweight` and `transparency` are display rows once a
    /// later lane emits them.
    pub fn for_key(key: &str) -> Self {
        match key {
            "id" | "handle" | "type" | "layer" | "space" | "draw_order" | "instance"
            | "sub_element" | "status" => PropertyGroup::Entity,
            "text" | "height" | "font" | "shape" | "size" | "style" => PropertyGroup::Text,
            "fill" | "visible" | "color" | "linetype" | "lineweight" | "transparency" => {
                PropertyGroup::Display
            }
            _ => PropertyGroup::Geometry,
        }
    }
}

/// The editing control a future write path would use for a property row.
///
/// This is machine metadata, not display text. It says nothing about whether
/// the row is **currently** writable — see [`PropertyRow::editable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyKind {
    Text,
    Number,
    Enum,
    Bool,
}

impl PropertyKind {
    /// Stable machine key carried on the wire (`text`, `number`, …).
    pub const fn key(self) -> &'static str {
        match self {
            PropertyKind::Text => "text",
            PropertyKind::Number => "number",
            PropertyKind::Enum => "enum",
            PropertyKind::Bool => "bool",
        }
    }

    /// The control kind for a property key.
    pub fn for_key(key: &str) -> Self {
        match key {
            "closed" | "inverted" | "visible" => PropertyKind::Bool,
            "draw_order" | "length" | "vertices" | "bulges" | "radius" | "start_angle"
            | "sweep" | "ratio" | "degree" | "control_points" | "triangles" | "height" | "size"
            | "opaque_version" | "payload_bytes" | "boundary" | "measurement" | "display"
            | "loops" | "roots" | "lines" | "children" => PropertyKind::Number,
            "layer" | "linetype" | "color" | "lineweight" | "transparency" | "space" | "status" => {
                PropertyKind::Enum
            }
            _ => PropertyKind::Text,
        }
    }
}

/// One read-only property row for the properties panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyRow {
    /// Stable machine key (never translated): `id`, `handle`, `type`, …
    pub key: &'static str,
    /// Human-facing value; numbers are formatted by the caller's unit policy
    /// only where the value is a length, otherwise raw drawing units.
    ///
    /// `None` **iff** [`Self::mixed`] is `true`: at least one selected object
    /// has a different value. There is never a fabricated empty string.
    pub value: Option<String>,
    /// Stable panel group machine key (never translated).
    pub group: PropertyGroup,
    /// Whether the value differs across a multi-selection (`value` is `None`).
    pub mixed: bool,
    /// Whether a future write path can change this value for **this**
    /// selection. Read-side metadata only: nothing here writes.
    pub editable: bool,
    /// The control kind a future editor would use.
    pub kind: PropertyKind,
    /// Allowed values for an `enum` row, from the real drawing tables (sorted,
    /// capped at [`MAX_ENUM_OPTIONS`]). Empty for every other kind.
    pub options: Vec<String>,
}

impl PropertyRow {
    pub fn new(key: &'static str, value: impl Into<String>) -> Self {
        PropertyRow {
            key,
            value: Some(value.into()),
            group: PropertyGroup::for_key(key),
            mixed: false,
            editable: false,
            kind: PropertyKind::for_key(key),
            options: Vec::new(),
        }
    }

    /// A row whose value differs across the selection: no value, explicit marker.
    pub fn mixed(key: &'static str) -> Self {
        PropertyRow {
            key,
            value: None,
            group: PropertyGroup::for_key(key),
            mixed: true,
            editable: false,
            kind: PropertyKind::for_key(key),
            options: Vec::new(),
        }
    }

    /// A row in an explicit group rather than the key's default group.
    pub fn in_group(key: &'static str, value: impl Into<String>, group: PropertyGroup) -> Self {
        PropertyRow {
            group,
            ..PropertyRow::new(key, value)
        }
    }
}

/// A selection with well-defined identity.
///
/// Order is preserved (the order the user picked), and duplicates — including
/// the same entity reached through two different `InstancePath`s — are rejected
/// so the panel count is honest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionSet {
    refs: Vec<SelectionRef>,
}

impl SelectionSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from refs, dropping duplicates while keeping first-seen order.
    pub fn from_refs(refs: impl IntoIterator<Item = SelectionRef>) -> Self {
        let mut set = SelectionSet::new();
        for r in refs {
            set.insert(r);
        }
        set
    }

    /// Whether two refs denote the same selected object.
    pub fn same_object(a: &SelectionRef, b: &SelectionRef) -> bool {
        a.entity == b.entity && a.instance == b.instance && a.sub_element == b.sub_element
    }

    /// Insert one ref if it is not already selected. Returns whether it was added.
    pub fn insert(&mut self, reference: SelectionRef) -> bool {
        if self
            .refs
            .iter()
            .any(|existing| Self::same_object(existing, &reference))
        {
            return false;
        }
        self.refs.push(reference);
        true
    }

    /// Remove one ref (exact identity). Returns whether it was present.
    pub fn remove(&mut self, reference: &SelectionRef) -> bool {
        let before = self.refs.len();
        self.refs
            .retain(|existing| !Self::same_object(existing, reference));
        self.refs.len() != before
    }

    /// Toggle one ref's membership.
    pub fn toggle(&mut self, reference: SelectionRef) {
        if !self.remove(&reference) {
            self.insert(reference);
        }
    }

    pub fn contains(&self, reference: &SelectionRef) -> bool {
        self.refs
            .iter()
            .any(|existing| Self::same_object(existing, reference))
    }

    pub fn replace(&mut self, refs: impl IntoIterator<Item = SelectionRef>) {
        self.refs = SelectionSet::from_refs(refs).refs;
    }

    pub fn clear(&mut self) {
        self.refs.clear();
    }

    /// Keep only the refs for which `keep` returns `true`, preserving order.
    ///
    /// Used to reconcile a session selection with the authoritative document
    /// after an edit that removed entities: the surviving refs keep their
    /// pick order, and only the removed ones are dropped (never a blanket
    /// clear).
    pub fn retain(&mut self, mut keep: impl FnMut(&SelectionRef) -> bool) {
        self.refs.retain(|reference| keep(reference));
    }

    pub fn len(&self) -> usize {
        self.refs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }

    pub fn refs(&self) -> &[SelectionRef] {
        &self.refs
    }

    /// A stable, human-facing identity of one ref that distinguishes INSERT
    /// instances and sub-elements. Used for the panel and, critically, for
    /// "same object?" comparisons a UI can display.
    pub fn identity_label(reference: &SelectionRef) -> String {
        let mut label = format!("{}", reference.entity.0);
        if !reference.instance.0.is_empty() {
            let path: Vec<String> = reference
                .instance
                .0
                .iter()
                .map(|e| e.0.to_string())
                .collect();
            label.push_str(&format!(" @ [{}]", path.join(" > ")));
        }
        if let Some(sub) = &reference.sub_element {
            label.push_str(&format!(" #{}", sub.source_key));
        }
        label
    }
}

/// Read-only projection of a selection for the properties panel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectionProperties {
    pub count: usize,
    /// Property rows of the selection.
    ///
    /// For one selected object this is the full ordered row set. For a
    /// multi-selection it is the **intersection**: every key present on all
    /// selected objects, in the first selected object's order, with a shared
    /// value or an explicit mixed marker. Empty only when nothing is selected.
    pub rows: Vec<PropertyRow>,
    /// Keys whose values differ across the selection (multi-select); exactly
    /// the keys of rows with [`PropertyRow::mixed`] set, in row order.
    pub mixed_keys: Vec<&'static str>,
    /// The **round-trippable** current string of a single selected, directly
    /// addressable text entity (ADR 0020 amendment B; ADR-0021 D4 extends it to
    /// TEXT, MTEXT and MULTILEADER text), or `None` for every other selection.
    ///
    /// This is deliberately **not** one of the display [`PropertyRow`]s: those
    /// values are formatted for presentation and cannot be written back. An empty
    /// selection, a multi-selection, a non-text entity, a content-less
    /// MULTILEADER, a block-instance member (instance path) and a sub-element all
    /// report `None` — text editing is addressed by entity identity only and
    /// never reaches into block internals. The value is the stored **raw**
    /// string: `\P` is not interpreted here.
    pub text: Option<String>,
    /// Whether [`Self::text`] is a **multi-line** text target: its source class
    /// (`AcDbMText`, or a `MultiLeader` with text content) treats a line break
    /// as a paragraph break. `false` for `AcDbText`, and always `false` whenever
    /// [`Self::text`] is `None` (multi-selection, empty selection, unresolved
    /// entity, instance path, sub-element, content-less leader, non-text).
    ///
    /// This is a **capability of the current text target**, not a display
    /// property: the docked panel and the popup must apply the same rule from
    /// this single derived value (and from the corresponding status field)
    /// instead of inspecting a display row such as `type`.
    pub text_multiline: bool,
    /// `true` when the selection is empty, so the UI shows an explicit empty
    /// state instead of fabricated rows.
    pub empty: bool,
}

impl SelectionProperties {
    /// Extract properties for a selection from the drawing database.
    ///
    /// - empty selection → `empty = true`, no rows;
    /// - exactly one selected entity that exists → full row set;
    /// - single selection that no longer resolves (deleted/foreign) → explicit
    ///   `status = unresolved` row, never a fake entity;
    /// - multiple selected → the intersection of their row keys; a key whose
    ///   value differs across refs becomes a mixed row (`value = None`,
    ///   `mixed = true`) and appears in `mixed_keys`. Keys present on only some
    ///   refs are not listed — the panel never invents blanks.
    pub fn extract(database: &DrawingDatabase, selection: &SelectionSet) -> Self {
        if selection.is_empty() {
            return SelectionProperties {
                count: 0,
                rows: Vec::new(),
                mixed_keys: Vec::new(),
                text: None,
                // Nothing editable means no text capability at all.
                text_multiline: false,
                empty: true,
            };
        }

        if selection.len() == 1 {
            let reference = &selection.refs()[0];
            let (rows, text, text_multiline) = match database.entity(reference.entity) {
                Some(entity) => {
                    let rows = entity_property_rows(database, reference, entity);
                    match editable_text(reference, entity) {
                        Some((text, multiline)) => (rows, Some(text), multiline),
                        None => (rows, None, false),
                    }
                }
                None => (
                    vec![
                        PropertyRow::new("id", reference.entity.0.to_string()),
                        PropertyRow::new("status", "unresolved"),
                    ],
                    None,
                    // An unresolved entity has no text capability to report.
                    false,
                ),
            };
            return SelectionProperties {
                count: 1,
                rows,
                mixed_keys: Vec::new(),
                text,
                text_multiline,
                empty: false,
            };
        }

        // Multi-select: keep only the keys every ref reports (intersection),
        // then collapse equal values and mark the rest as mixed.
        let per_ref: Vec<Vec<PropertyRow>> = selection
            .refs()
            .iter()
            .map(|reference| match database.entity(reference.entity) {
                Some(entity) => entity_property_rows(database, reference, entity),
                None => vec![PropertyRow::new("status", "unresolved")],
            })
            .collect();
        let mut rows: Vec<PropertyRow> = Vec::new();
        for first_row in &per_ref[0] {
            let key = first_row.key;
            let Some(members) = per_ref
                .iter()
                .map(|rows| rows.iter().find(|row| row.key == key))
                .collect::<Option<Vec<&PropertyRow>>>()
            else {
                // Not on every selected object: the intersection excludes it.
                continue;
            };
            let first_value = members[0].value.clone();
            let mixed = first_value.is_none() || members.iter().any(|row| row.value != first_value);
            let mut row = match first_value {
                Some(value) if !mixed => PropertyRow::new(key, value),
                _ => PropertyRow::mixed(key),
            };
            // Group and kind follow the key's stable mapping (the first ref's
            // row is authoritative for the per-geometry overrides, e.g. `style`).
            row.group = first_row.group;
            row.kind = first_row.kind;
            row.options = enum_options(database, key);
            // Editable only when **every** selected ref supports the write path.
            row.editable = members.iter().all(|row| row.editable);
            rows.push(row);
        }
        let mixed_keys: Vec<&'static str> = rows
            .iter()
            .filter(|row| row.mixed)
            .map(|row| row.key)
            .collect();
        SelectionProperties {
            count: selection.len(),
            rows,
            mixed_keys,
            text: None,
            // A multi-selection has no single text target, so no capability.
            text_multiline: false,
            empty: false,
        }
    }
}

/// The round-trippable text of one selected entity and whether its source class
/// is multiline, or `None`.
///
/// Only a directly addressed text-bearing entity qualifies: `TEXTEDIT` replaces
/// the string of an **entity**, so a block-instance member (an instance path) or
/// a sub-element is not editable here (ADR 0020 amendment B). The value is the
/// stored string verbatim, never a formatted display row.
fn editable_text(reference: &SelectionRef, entity: &DbEntity) -> Option<(String, bool)> {
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return None;
    }
    editable_text_source(entity).map(|(text, multiline)| (text.to_string(), multiline))
}

/// The raw, writable string of one directly addressable text-bearing entity and
/// whether its source class treats a line break as a paragraph break (ADR-0021
/// D4).
///
/// Qualifying kinds are a [`SemanticGeometry::Text`] (`AcDbText`: single-line;
/// `AcDbMText`: multiline) and a [`SemanticGeometry::MultiLeader`] whose content
/// is [`MLeaderContent::Text`] (always multiline). A content-less leader has no
/// string to replace, and block/tolerance content never reaches the first-class
/// leader at all, so both report `None`. Instance paths and sub-elements are the
/// caller's check: this helper looks at the entity only.
pub(crate) fn editable_text_source(entity: &DbEntity) -> Option<(&str, bool)> {
    match &entity.geometry {
        SemanticGeometry::Text { text, .. } => {
            Some((text.as_str(), entity.object.type_key == "AcDbMText"))
        }
        SemanticGeometry::MultiLeader(leader) if leader.content() == MLeaderContent::Text => {
            Some((leader.text().string.as_str(), true))
        }
        _ => None,
    }
}

/// Property rows for one resolved entity: identity + geometry basics.
pub fn entity_property_rows(
    database: &DrawingDatabase,
    reference: &SelectionRef,
    entity: &DbEntity,
) -> Vec<PropertyRow> {
    let mut rows = vec![
        PropertyRow::new("id", entity.id.0.to_string()),
        PropertyRow::new("handle", identity_handle(entity)),
        PropertyRow::new("type", entity.object.type_key.clone()),
        PropertyRow::new("layer", layer_label(database, entity.layer)),
        PropertyRow::new("space", space_label(&entity.space)),
        PropertyRow::new("draw_order", entity.draw_order.to_string()),
    ];
    if !reference.instance.0.is_empty() {
        rows.push(PropertyRow::new(
            "instance",
            SelectionSet::identity_label(reference),
        ));
    }
    if let Some(sub) = &reference.sub_element {
        rows.push(PropertyRow::new("sub_element", sub.source_key.clone()));
    }
    for row in &mut rows {
        if row.key == "layer" {
            row.editable = property_row_editable(database, reference, entity, "layer");
            row.options = enum_options(database, "layer");
        }
    }
    rows.extend(geometry_property_rows(&entity.geometry));
    rows
}

/// The sorted, capped option list for an `enum` property key.
///
/// Only real drawing tables are used: `layer` → the drawing's layer names,
/// `linetype` → its linetype names. Every other key (including `color`,
/// `lineweight` and `transparency`, which are ACI/value pickers rather than
/// named tables) reports no options. Names are deduplicated and sorted so the
/// same drawing always yields the same order.
pub fn enum_options(database: &DrawingDatabase, key: &str) -> Vec<String> {
    let names: BTreeSet<&str> = match key {
        "layer" => database.layers().map(|layer| layer.name.as_str()).collect(),
        "linetype" => database
            .linetypes()
            .map(|linetype| linetype.name.as_str())
            .collect(),
        _ => return Vec::new(),
    };
    names
        .into_iter()
        .take(MAX_ENUM_OPTIONS)
        .map(str::to_string)
        .collect()
}

/// Whether the write path can change `key` for this selected object.
///
/// A row is editable when the write target is a directly addressable top-level
/// entity (the same rule [`editable_text`] uses: no instance path, no
/// sub-element) **and** the key is one of the five properties
/// [`crate::property_value::SUPPORTED_KEYS`] accepts: `layer`, `color`,
/// `linetype`, `lineweight`, `transparency`.
///
/// The current value does **not** restrict editability: the write path accepts a
/// symbolic value (`ByLayer`/`ByBlock`) as well as a concrete one, so a row
/// whose stored value is symbolic is still writable (and a concrete value can be
/// switched back to symbolic). Every other key is `false`: text content has its
/// own edit path (ADR 0020 amendment B) and no other property is writable.
pub fn property_row_editable(
    _database: &DrawingDatabase,
    reference: &SelectionRef,
    _entity: &DbEntity,
    key: &str,
) -> bool {
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return false;
    }
    crate::property_value::is_supported_key(key)
}

/// The stored handle, or an explicit "none" — never a synthesized number.
fn identity_handle(entity: &DbEntity) -> String {
    entity
        .object
        .source_handle
        .clone()
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| "none".to_string())
}

fn layer_label(database: &DrawingDatabase, layer: cad_domain::LayerId) -> String {
    match database.layer(layer) {
        Some(l) => format!("{} ({})", l.name, layer.0),
        None => format!("unresolved ({})", layer.0),
    }
}

fn space_label(space: &SpaceId) -> String {
    match space {
        SpaceId::Model => "model".to_string(),
        SpaceId::Paper(layout) => format!("paper({})", layout.0),
        SpaceId::Block(block) => format!("block({})", block.0),
    }
}

/// Geometry-specific basics for each supported semantic geometry.
///
/// Only true, computed values are emitted. Curved lengths are not invented
/// here; callers that need an exact length use the measure engine.
fn geometry_property_rows(geometry: &SemanticGeometry) -> Vec<PropertyRow> {
    match geometry {
        SemanticGeometry::Line { start, end } => vec![
            PropertyRow::new("start", point_label(*start)),
            PropertyRow::new("end", point_label(*end)),
            PropertyRow::new("length", format!("{:.6}", distance(*start, *end))),
        ],
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => vec![
            PropertyRow::new("vertices", points.len().to_string()),
            PropertyRow::new("closed", closed.to_string()),
            PropertyRow::new("bulges", bulges.len().to_string()),
        ],
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => vec![
            PropertyRow::new("center", point_label(*center)),
            PropertyRow::new("normal", point_label(*normal)),
            PropertyRow::new("radius", format!("{radius:.6}")),
        ],
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => vec![
            PropertyRow::new("center", point_label(*center)),
            PropertyRow::new("radius", format!("{radius:.6}")),
            PropertyRow::new("start_angle", format!("{start:.6}")),
            PropertyRow::new("sweep", format!("{sweep:.6}")),
        ],
        SemanticGeometry::Ellipse {
            center,
            ratio,
            start,
            sweep,
            ..
        } => vec![
            PropertyRow::new("center", point_label(*center)),
            PropertyRow::new("ratio", format!("{ratio:.6}")),
            PropertyRow::new("start_angle", format!("{start:.6}")),
            PropertyRow::new("sweep", format!("{sweep:.6}")),
        ],
        SemanticGeometry::Spline {
            degree,
            control_points,
            ..
        } => vec![
            PropertyRow::new("degree", degree.to_string()),
            PropertyRow::new("control_points", control_points.len().to_string()),
        ],
        SemanticGeometry::Point(p) => vec![PropertyRow::new("position", point_label(*p))],
        SemanticGeometry::Mesh(mesh) => vec![
            PropertyRow::new("vertices", mesh.vertices.len().to_string()),
            PropertyRow::new("triangles", mesh.triangle_count().to_string()),
        ],
        SemanticGeometry::Insert { block, .. } => {
            vec![PropertyRow::new("block", block.0.to_string())]
        }
        SemanticGeometry::Text {
            text,
            position,
            height,
            font,
            ..
        } => vec![
            PropertyRow::new("text", text.clone()),
            PropertyRow::new("position", point_label(*position)),
            PropertyRow::new("height", format!("{height:.6}")),
            PropertyRow::new(
                "font",
                font.clone().unwrap_or_else(|| "unknown".to_string()),
            ),
        ],
        SemanticGeometry::Shape {
            shape_name,
            code,
            position,
            size,
            font,
            ..
        } => vec![
            PropertyRow::new(
                "shape",
                if shape_name.is_empty() {
                    code.to_string()
                } else {
                    shape_name.clone()
                },
            ),
            PropertyRow::new("position", point_label(*position)),
            PropertyRow::new("size", format!("{size:.6}")),
            PropertyRow::new(
                "font",
                font.clone().unwrap_or_else(|| "unknown".to_string()),
            ),
        ],
        SemanticGeometry::Opaque {
            type_key,
            version,
            payload,
        } => vec![
            PropertyRow::new("opaque_type", type_key.clone()),
            PropertyRow::new("opaque_version", version.to_string()),
            PropertyRow::new("payload_bytes", payload.len().to_string()),
        ],
        // True, stored metadata only: no geometric length is invented for an
        // image (its world extent lives in the placement vectors).
        SemanticGeometry::Image {
            pixels,
            file,
            visible,
            ..
        } => vec![
            PropertyRow::new("pixels", format!("{} x {}", pixels[0], pixels[1])),
            PropertyRow::new("file", file.clone().unwrap_or_else(|| "none".to_string())),
            PropertyRow::new("visible", visible.to_string()),
        ],
        SemanticGeometry::Mask { boundary, inverted } => vec![
            PropertyRow::new("boundary", boundary.len().to_string()),
            PropertyRow::new("inverted", inverted.to_string()),
        ],
        SemanticGeometry::Compound(children) => {
            vec![PropertyRow::new("children", children.len().to_string())]
        }
        SemanticGeometry::Dimension(dimension) => vec![
            PropertyRow::new("subtype", format!("{:?}", dimension.kind())),
            PropertyRow::new("measurement", format!("{:.6}", dimension.measurement())),
            PropertyRow::new("display", dimension.display().len().to_string()),
        ],
        SemanticGeometry::Hatch(hatch) => {
            let fill = match hatch.fill() {
                HatchFill::Solid => "solid".to_string(),
                HatchFill::Pattern(pattern) => format!("pattern {}", pattern.name),
                HatchFill::Gradient(gradient) => format!("gradient {}", gradient.name()),
            };
            vec![
                PropertyRow::new("loops", hatch.loops().len().to_string()),
                PropertyRow::in_group(
                    "style",
                    format!("{:?}", hatch.style()),
                    PropertyGroup::Display,
                ),
                PropertyRow::new("fill", fill),
                PropertyRow::new("display", hatch.display().len().to_string()),
            ]
        }
        // Stored metadata only: the derived display is not re-described here.
        SemanticGeometry::MultiLeader(multileader) => vec![
            PropertyRow::in_group(
                "style",
                multileader.style_name().to_string(),
                PropertyGroup::Text,
            ),
            PropertyRow::new("roots", multileader.roots().len().to_string()),
            PropertyRow::new("lines", multileader.lines().len().to_string()),
            PropertyRow::new("display", multileader.display().len().to_string()),
        ],
    }
}

fn point_label(p: Point3) -> String {
    format!("{:.6}, {:.6}, {:.6}", p.x, p.y, p.z)
}

fn distance(a: Point3, b: Point3) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Convenience: build a model-space selection ref for a plain entity (no
/// instance path, no sub-element).
pub fn model_ref(document: cad_domain::DocumentId, entity: EntityId) -> SelectionRef {
    SelectionRef {
        document,
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }
}

#[cfg(test)]
#[path = "tests/selection_tests.rs"]
mod tests;
