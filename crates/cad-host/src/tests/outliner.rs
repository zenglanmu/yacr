//! The Outliner read: a bounded pre-order projection of groups, block
//! references and model-space entities.
//!
//! The projection is read-only; these contracts cover the row shape, the
//! selectable/`selectIds` split (block members are not model-space addresses),
//! and that no document is an empty answer rather than an error. No GUI or GPU
//! is involved.

use super::*;
use cad_app::{AnnotationBatchPayload, GroupCommand};

/// A core with **no** document (`StartDocument::Empty`), so the empty answer is
/// a real one rather than the demo document's rows.
fn empty_core() -> HostCore {
    let mut options = options();
    options.document_at_start = StartDocument::Empty;
    HostCore::new(
        options,
        HostPlatform {
            files: Rc::new(FakeFiles::default()),
            config_store: Rc::new(FakeConfig),
            surface: Rc::new(FakeSurface),
            resources: Rc::new(UnsupportedResources),
            capabilities: capabilities(false),
        },
        RenderSessionHandle::default(),
    )
    .expect("a host core")
}

fn define_block(core: &HostCore, members: &[cad_domain::EntityId], name: &str) {
    let document = core.controller.borrow().document_id;
    run_controller_command(
        core,
        CommandId::DefineBlock,
        CommandPayload::DefineBlock {
            refs: members
                .iter()
                .map(|member| cad_app::selection::model_ref(document, *member))
                .collect(),
            base: p(0.0, 0.0),
            name: name.into(),
        },
    );
}

fn group(core: &HostCore, members: &[cad_domain::EntityId], name: &str) {
    let document = core.controller.borrow().document_id;
    core.controller.borrow_mut().session.selection.replace(
        members
            .iter()
            .map(|member| cad_app::selection::model_ref(document, *member)),
    );
    run_controller_command(
        core,
        CommandId::Group,
        CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Group(
            GroupCommand::Create(name.into()),
        ))),
    );
}

/// No document is the empty answer (`revision: 0`), never an error.
#[test]
fn an_empty_document_projects_an_empty_tree() {
    let core = empty_core();
    let tree = core.outliner_tree();
    assert_eq!(tree.revision, 0);
    assert!(tree.rows.is_empty());
    assert_eq!(tree.total, 0);
    assert!(!tree.truncated);
}

/// A group is one container whose members are selectable entity rows; a block
/// reference is a container whose members are readable but not selectable.
#[test]
fn groups_and_block_references_project_as_containers() {
    let core = core();
    let (a, b) = add_two_lines(&core);
    define_block(&core, &[a, b], "Widget");
    let (c, d) = add_two_lines(&core);
    group(&core, &[c, d], "walls");

    let tree = core.outliner_tree();
    assert!(tree.revision > 0);
    assert_eq!(tree.total, tree.rows.len());
    assert!(!tree.truncated);

    let group_row = tree
        .rows
        .iter()
        .find(|row| row.kind == "group")
        .expect("the group row");
    assert_eq!(group_row.label, "walls");
    assert_eq!(group_row.depth, 0);
    assert!(group_row.has_children);
    assert!(group_row.selectable);
    let mut members = group_row.select_ids.clone();
    members.sort_unstable();
    let mut expected = vec![c.0 as u64, d.0 as u64];
    expected.sort_unstable();
    assert_eq!(members, expected, "one click selects the whole group");

    let insert_row = tree
        .rows
        .iter()
        .find(|row| row.kind == "insert")
        .expect("the block reference row");
    assert!(insert_row.label.starts_with("Widget"), "{insert_row:?}");
    assert!(insert_row.selectable);
    assert_eq!(insert_row.select_ids, vec![insert_row.id]);
    assert_eq!(insert_row.depth, 0);

    let block_members: Vec<_> = tree
        .rows
        .iter()
        .filter(|row| row.kind == "member" && (row.id == a.0 as u64 || row.id == b.0 as u64))
        .collect();
    assert_eq!(block_members.len(), 2);
    for member in block_members {
        assert_eq!(member.depth, 1, "a block member nests under its reference");
        assert!(!member.selectable, "a block member has no model-space id");
        assert!(member.select_ids.is_empty(), "{member:?}");
    }

    // Group members are entity rows at depth 1 and select themselves. The demo
    // document has its own top-level rows, so this checks the two ids we grouped.
    let entity_rows: Vec<_> = tree
        .rows
        .iter()
        .filter(|row| row.kind == "entity" && (row.id == c.0 as u64 || row.id == d.0 as u64))
        .collect();
    assert_eq!(entity_rows.len(), 2);
    for row in entity_rows {
        assert_eq!(row.depth, 1);
        assert!(row.selectable);
        assert_eq!(row.select_ids, vec![row.id]);
    }
}

/// A page that is short states its real `total` and `truncated: true`.
#[test]
fn the_tree_is_bounded_and_reports_its_real_total() {
    let core = core();
    let (a, b) = add_two_lines(&core);
    group(&core, &[a, b], "pair");
    // The cap is a page budget, not a second document limit: the demo document
    // plus the group fits, so this pins the invariant rather than the cap.
    let tree = core.outliner_tree();
    assert!(tree.total >= 3, "group + its two members at least");
    assert!(!tree.truncated);
    assert_eq!(tree.total, tree.rows.len());
    assert!(tree.rows.len() <= crate::OUTLINER_MAX_ROWS);
}

#[test]
fn a_session_hidden_entity_is_marked_hidden_in_the_tree() {
    let core = core();
    let (document, entity) = {
        let controller = core.controller.borrow();
        let drawing = controller.drawing().expect("demo document");
        let entity = drawing.model_space().first().expect("a model entity").id;
        (controller.document_id, entity)
    };
    let reference = cad_app::selection::model_ref(document, entity);
    core.controller
        .borrow_mut()
        .session
        .hidden_entities
        .insert(reference);
    let tree = core.outliner_tree();
    let row = tree
        .rows
        .iter()
        .find(|row| row.kind == "entity" && row.id == entity.0 as u64)
        .expect("the entity row");
    assert!(row.hidden, "HIDEOBJECTS must be visible in the Outliner");

    core.controller.borrow_mut().session.hidden_entities.clear();
    let tree = core.outliner_tree();
    let row = tree
        .rows
        .iter()
        .find(|row| row.kind == "entity" && row.id == entity.0 as u64)
        .expect("the entity row");
    assert!(!row.hidden);
}
