//! `AcDbGroup` export/import agreement through the real DXF writer.
//!
//! The name lives in the `ACAD_GROUP` dictionary (DXF does not repeat it in the
//! GROUP body), so these tests exercise the dictionary bridge as well as the
//! member-handle mapping. This is internal consistency (our writer → our
//! reader), never a third-party AutoCAD compatibility claim.

use super::*;
use cad_db::{GroupRow, GroupSeq, Layout};

fn line(id: u128, space: SpaceId, draw_order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Line {
            start: Point3 {
                x: id as f64,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: id as f64,
                y: 1.0,
                z: 0.0,
            },
        },
        draw_order,
    }
}

fn database_with_group(members: Vec<EntityId>, paper_member: Option<EntityId>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(0),
            name: "Model".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder.insert_entity(line(1, SpaceId::Model, 0)).unwrap();
    builder.insert_entity(line(2, SpaceId::Model, 1)).unwrap();
    if let Some(paper) = paper_member {
        builder
            .insert_entity(line(paper.0, SpaceId::Paper(LayoutId(1)), 2))
            .unwrap();
    }
    let mut row = GroupRow::new(GroupSeq(1), "Walls", members);
    row.description = "exterior".into();
    row.selectable = false;
    builder.set_groups(vec![row]).unwrap();
    builder.finish().unwrap()
}

#[test]
fn a_group_survives_dxf_export_and_import() {
    let database = database_with_group(vec![EntityId(1), EntityId(2)], None);
    let (bytes, report) = export_db(&database);
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.code.contains("group")),
        "both members are written; no group loss: {:?}",
        report.entries
    );
    let loaded = import_bytes(&bytes);
    let groups: Vec<_> = loaded.groups().collect();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "Walls");
    assert_eq!(groups[0].description, "exterior");
    assert!(!groups[0].selectable);
    let members: Vec<u128> = groups[0].members.iter().map(|id| id.0).collect();
    assert_eq!(members.len(), 2);
    // The imported ids are allocation-order ids; the source handles prove both
    // model lines came back and are group members.
    let handles: Vec<String> = members
        .iter()
        .filter_map(|id| loaded.entity(EntityId(*id)))
        .filter_map(|entity| entity.object.source_handle.clone())
        .collect();
    assert_eq!(handles.len(), 2);
}

#[test]
fn a_paper_space_member_is_named_and_the_group_is_still_written() {
    let database = database_with_group(vec![EntityId(1), EntityId(3)], Some(EntityId(3)));
    let (bytes, report) = export_db(&database);
    let dropped: Vec<&str> = report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.group_member_not_written")
        .map(|entry| entry.code.as_str())
        .collect();
    assert_eq!(dropped.len(), 1, "the paper member is named once");
    let loaded = import_bytes(&bytes);
    let groups: Vec<_> = loaded.groups().collect();
    assert_eq!(groups.len(), 1, "the group itself is never dropped");
    assert_eq!(groups[0].members.len(), 1);
}

#[test]
fn an_empty_group_is_still_written() {
    // A group whose only member was not written keeps an empty member list but
    // still exists in the output; the loss is reported, not hidden.
    let database = database_with_group(Vec::new(), None);
    let (bytes, _) = export_db(&database);
    let loaded = import_bytes(&bytes);
    let groups: Vec<_> = loaded.groups().collect();
    assert_eq!(groups.len(), 1);
    assert!(groups[0].members.is_empty());
}
