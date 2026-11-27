//! The Materials tray read: the document's own `tables/materials.json` rows.
//!
//! Host content only: the read projects what `PAINT` wrote and never paints.
//! Contract level: no GUI, no GPU.

use super::*;
use cad_app::{ExtendedCommandId, SelectionSet};
use cad_domain::{InstancePath, SelectionRef};

#[test]
fn the_materials_read_lists_what_paint_wrote() {
    let core = core();
    assert!(
        core.materials().rows.is_empty(),
        "the demo document starts without materials"
    );
    let (document, entity) = {
        let controller = core.controller.borrow();
        let drawing = controller.drawing().expect("the demo document");
        let entity = drawing
            .model_space()
            .last()
            .map(|entity| entity.id)
            .expect("the demo document has an entity");
        (controller.document_id, entity)
    };
    core.controller.borrow_mut().session.selection = SelectionSet::from_refs([SelectionRef {
        document,
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    core.command(
        CommandId::Extended(ExtendedCommandId::Paint),
        CommandPayload::PaintMaterial("Red".into()),
    )
    .expect("the paint writes the first material row");

    let list = core.materials();
    assert_eq!(list.total, list.rows.len(), "the table is not truncated");
    assert_eq!(list.rows.len(), 1);
    let row = &list.rows[0];
    assert_eq!(row.name, "Red");
    assert_eq!(row.color, [178, 34, 34]);
    assert_eq!(row.transparency, 0.0);
    assert!(!list.truncated);
}

#[test]
fn painting_another_builtin_keeps_both_rows() {
    let core = core();
    let (document, entity) = {
        let controller = core.controller.borrow();
        let drawing = controller.drawing().expect("the demo document");
        let entity = drawing
            .model_space()
            .last()
            .map(|entity| entity.id)
            .expect("the demo document has an entity");
        (controller.document_id, entity)
    };
    core.controller.borrow_mut().session.selection = SelectionSet::from_refs([SelectionRef {
        document,
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    for name in ["Red", "Blue"] {
        core.command(
            CommandId::Extended(ExtendedCommandId::Paint),
            CommandPayload::PaintMaterial(name.into()),
        )
        .expect("paint");
    }
    let list = core.materials();
    let names: Vec<&str> = list.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        names,
        ["Red", "Blue"],
        "both rows are listed in table order"
    );
    assert_eq!(list.total, 2);
}
