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

use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::{EntityId, InstancePath, Point3, SelectionRef, SemanticGeometry, SpaceId};
use std::collections::BTreeSet;

/// One read-only property row for the properties panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyRow {
    /// Stable machine key (never translated): `id`, `handle`, `type`, …
    pub key: &'static str,
    /// Human-facing value; numbers are formatted by the caller's unit policy
    /// only where the value is a length, otherwise raw drawing units.
    pub value: String,
}

impl PropertyRow {
    pub fn new(key: &'static str, value: impl Into<String>) -> Self {
        PropertyRow {
            key,
            value: value.into(),
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
    /// Rows describing the single selected object; empty when `count != 1`.
    pub rows: Vec<PropertyRow>,
    /// Keys whose values differ across the selection (multi-select).
    pub mixed_keys: Vec<&'static str>,
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
    /// - multiple selected → `mixed_keys` lists keys whose values differ, and
    ///   `rows` stays empty (the panel shows "N selected" + mixed keys).
    pub fn extract(database: &DrawingDatabase, selection: &SelectionSet) -> Self {
        if selection.is_empty() {
            return SelectionProperties {
                count: 0,
                rows: Vec::new(),
                mixed_keys: Vec::new(),
                empty: true,
            };
        }

        if selection.len() == 1 {
            let reference = &selection.refs()[0];
            let rows = match database.entity(reference.entity) {
                Some(entity) => entity_property_rows(database, reference, entity),
                None => vec![
                    PropertyRow::new("id", reference.entity.0.to_string()),
                    PropertyRow::new("status", "unresolved"),
                ],
            };
            return SelectionProperties {
                count: 1,
                rows,
                mixed_keys: Vec::new(),
                empty: false,
            };
        }

        // Multi-select: compare the surface property keys of every member.
        let per_ref: Vec<Vec<PropertyRow>> = selection
            .refs()
            .iter()
            .map(|reference| match database.entity(reference.entity) {
                Some(entity) => entity_property_rows(database, reference, entity),
                None => vec![PropertyRow::new("status", "unresolved")],
            })
            .collect();
        let mut mixed = BTreeSet::new();
        let mut keys: Vec<&'static str> = Vec::new();
        for rows in &per_ref {
            for row in rows {
                if !keys.contains(&row.key) {
                    keys.push(row.key);
                }
            }
        }
        for key in &keys {
            let value =
                |rows: &[PropertyRow]| rows.iter().find(|r| r.key == *key).map(|r| r.value.clone());
            let first = value(&per_ref[0]);
            if per_ref.iter().any(|rows| value(rows) != first) {
                mixed.insert(*key);
            }
        }
        SelectionProperties {
            count: selection.len(),
            rows: Vec::new(),
            mixed_keys: mixed.into_iter().collect(),
            empty: false,
        }
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
    rows.extend(geometry_property_rows(&entity.geometry));
    rows
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
        SemanticGeometry::Opaque {
            type_key,
            version,
            payload,
        } => vec![
            PropertyRow::new("opaque_type", type_key.clone()),
            PropertyRow::new("opaque_version", version.to_string()),
            PropertyRow::new("payload_bytes", payload.len().to_string()),
        ],
        SemanticGeometry::Compound(children) => {
            vec![PropertyRow::new("children", children.len().to_string())]
        }
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
mod tests {
    use super::*;
    use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_domain::{
        BlockId, DatabaseId, DocumentId, EntityId, InstancePath, LayerId, ObjectId, Revision,
        StyleId, Transform3,
    };

    fn entity(id: u128, geometry: SemanticGeometry, layer: LayerId) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbEntity".into(),
                revision: Revision(0),
                source_handle: Some(format!("{id:X}")),
            },
            id: EntityId(id),
            layer,
            space: SpaceId::Model,
            geometry,
            draw_order: id as i64,
        }
    }

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn database() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(20)],
        })
        .unwrap();
        b.insert_entity(entity(
            20,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
            LayerId(0),
        ))
        .unwrap();
        b.insert_entity(entity(
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(3.0, 4.0),
            },
            LayerId(0),
        ))
        .unwrap();
        b.insert_entity(entity(
            2,
            SemanticGeometry::Circle {
                center: point(1.0, 2.0),
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                radius: 2.5,
            },
            LayerId(0),
        ))
        .unwrap();
        b.insert_entity(entity(
            3,
            SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::translation(point(10.0, 0.0)),
            },
            LayerId(0),
        ))
        .unwrap();
        b.finish().unwrap()
    }

    fn ref_of(document: DocumentId, entity: EntityId) -> SelectionRef {
        model_ref(document, entity)
    }

    #[test]
    fn selection_distinguishes_insert_instances_via_instance_path() {
        let doc = DocumentId(1);
        // Same block entity reached through two different placements.
        let a = SelectionRef {
            document: doc,
            entity: EntityId(20),
            instance: InstancePath(vec![EntityId(3)]),
            sub_element: None,
        };
        let b = SelectionRef {
            document: doc,
            entity: EntityId(20),
            instance: InstancePath(vec![EntityId(4)]),
            sub_element: None,
        };
        let mut set = SelectionSet::new();
        assert!(set.insert(a.clone()));
        assert!(set.insert(b.clone()));
        assert_eq!(set.len(), 2, "different instances are separate objects");

        // The identical instance is a duplicate.
        assert!(!set.insert(a.clone()));
        assert_eq!(set.len(), 2);

        // Identity labels differ.
        assert_ne!(
            SelectionSet::identity_label(&a),
            SelectionSet::identity_label(&b)
        );

        // A sub-element of the same instance is again distinct.
        let sub = SelectionRef {
            document: doc,
            entity: EntityId(20),
            instance: InstancePath(vec![EntityId(3)]),
            sub_element: Some(cad_domain::SubElementId {
                source_key: "face0".into(),
                topology_revision: Revision(0),
            }),
        };
        assert!(set.insert(sub));
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn toggle_and_remove_are_identity_exact() {
        let doc = DocumentId(1);
        let a = ref_of(doc, EntityId(1));
        let mut set = SelectionSet::new();
        set.toggle(a.clone());
        assert!(set.contains(&a));
        set.toggle(a.clone());
        assert!(!set.contains(&a));
        set.insert(a.clone());
        assert!(set.remove(&a));
        assert!(!set.remove(&a));
    }

    #[test]
    fn empty_selection_is_explicitly_empty() {
        let db = database();
        let props = SelectionProperties::extract(&db, &SelectionSet::new());
        assert!(props.empty);
        assert_eq!(props.count, 0);
        assert!(props.rows.is_empty());
    }

    #[test]
    fn single_line_selection_reports_identity_and_geometry() {
        let db = database();
        let set = SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(1))]);
        let props = SelectionProperties::extract(&db, &set);
        assert!(!props.empty);
        assert_eq!(props.count, 1);
        let row = |key: &str| {
            props
                .rows
                .iter()
                .find(|r| r.key == key)
                .map(|r| r.value.clone())
        };
        assert_eq!(row("id").as_deref(), Some("1"));
        assert_eq!(row("handle").as_deref(), Some("1"));
        assert_eq!(row("type").as_deref(), Some("AcDbEntity"));
        assert_eq!(row("layer").as_deref(), Some("0 (0)"));
        assert_eq!(row("space").as_deref(), Some("model"));
        // Line length 3-4-5.
        assert_eq!(row("length").as_deref(), Some("5.000000"));
    }

    #[test]
    fn circle_and_insert_report_geometry_specific_rows() {
        let db = database();
        let circle = SelectionProperties::extract(
            &db,
            &SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(2))]),
        );
        let radius = circle
            .rows
            .iter()
            .find(|r| r.key == "radius")
            .unwrap()
            .value
            .clone();
        assert_eq!(radius, "2.500000");

        let insert = SelectionProperties::extract(
            &db,
            &SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(3))]),
        );
        let block = insert
            .rows
            .iter()
            .find(|r| r.key == "block")
            .unwrap()
            .value
            .clone();
        assert_eq!(block, "0");
    }

    #[test]
    fn multi_select_reports_mixed_keys_instead_of_a_single_value() {
        let db = database();
        let set = SelectionSet::from_refs([
            ref_of(DocumentId(1), EntityId(1)), // Line
            ref_of(DocumentId(1), EntityId(2)), // Circle
        ]);
        let props = SelectionProperties::extract(&db, &set);
        assert_eq!(props.count, 2);
        assert!(props.rows.is_empty());
        // Geometry keys differ (line has `length`, circle has `radius`).
        assert!(props.mixed_keys.contains(&"length"));
        assert!(props.mixed_keys.contains(&"radius"));
        // Shared identity keys are not "mixed" when values agree; ids differ.
        assert!(props.mixed_keys.contains(&"id"));
    }

    #[test]
    fn unresolved_entity_is_reported_honestly() {
        let db = database();
        let set = SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(999))]);
        let props = SelectionProperties::extract(&db, &set);
        assert_eq!(props.count, 1);
        let status = props
            .rows
            .iter()
            .find(|r| r.key == "status")
            .map(|r| r.value.clone());
        assert_eq!(status.as_deref(), Some("unresolved"));
    }

    #[test]
    fn text_without_font_is_explicitly_unknown() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(2));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(entity(
            7,
            SemanticGeometry::Text {
                text: "标注".into(),
                position: point(0.0, 0.0),
                style: StyleId(0),
                height: 2.5,
                rotation: 0.0,
                font: None,
                h_align: cad_domain::TextAlignH::Left,
                v_align: cad_domain::TextAlignV::Baseline,
            },
            LayerId(0),
        ))
        .unwrap();
        let db = b.finish().unwrap();
        let props = SelectionProperties::extract(
            &db,
            &SelectionSet::from_refs([ref_of(DocumentId(2), EntityId(7))]),
        );
        let font = props
            .rows
            .iter()
            .find(|r| r.key == "font")
            .map(|r| r.value.clone());
        assert_eq!(font.as_deref(), Some("unknown"));
    }
}
