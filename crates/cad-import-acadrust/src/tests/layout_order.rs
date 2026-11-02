//! Layout display order and user-visible names (LAYOUT v1 core).
//!
//! The contract under test is the *bridge* from acadrust's `ObjectType::Layout`
//! to `cad-db`:
//!
//! * the database id is the sheet's tab position, not block-record order;
//! * the name is the LAYOUT object's display name, not `*Paper_Space0`;
//! * paper entities still land in the layout their block record maps to.
//!
//! The test constructs an in-memory `CadDocument` and runs the importer's
//! `ImporterBuilder` directly. That deliberately skips serialization: the
//! contract here is `read_layouts`/`read_entities`, and a writer/parser round
//! trip would add a second system whose ordering could mask (or manufacture)
//! the ordering bug. The same test shape is used elsewhere in this crate for
//! in-memory acadrust documents.

use super::*;

fn layout_handle(doc: &acadrust::CadDocument, name: &str) -> acadrust::Handle {
    doc.objects
        .iter()
        .find_map(|(handle, object)| match object {
            acadrust::objects::ObjectType::Layout(layout) if layout.name == name => Some(*handle),
            _ => None,
        })
        .unwrap_or_else(|| panic!("layout {name} exists"))
}

fn set_tab_order(doc: &mut acadrust::CadDocument, name: &str, order: i16) {
    let handle = layout_handle(doc, name);
    let Some(acadrust::objects::ObjectType::Layout(layout)) = doc.objects.get_mut(&handle) else {
        panic!("layout {name} is a Layout object");
    };
    layout.tab_order = order;
}

fn content_viewport(center: (f64, f64)) -> acadrust::entities::Viewport {
    let mut viewport = acadrust::entities::Viewport::new();
    viewport.id = 2;
    viewport.center = acadrust::types::Vector3::new(center.0, center.1, 0.0);
    viewport.width = 20.0;
    viewport.height = 10.0;
    viewport.view_height = 2000.0;
    viewport.view_direction = acadrust::types::Vector3::UNIT_Z;
    viewport.view_target = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
    viewport
}

fn add_sheet_content(doc: &mut acadrust::CadDocument, name: &str, center: (f64, f64), x: f64) {
    doc.add_entity_to_layout(
        EntityType::Viewport(Box::new(content_viewport(center))),
        name,
    )
    .expect("add content viewport");
    doc.add_entity_to_layout(
        EntityType::Line(acadrust::entities::Line::from_coords(
            x,
            0.0,
            0.0,
            x + 1.0,
            1.0,
            0.0,
        )),
        name,
    )
    .expect("add paper line");
}

/// Two sheets whose block-record creation order (`*Paper_Space`, then
/// `*Paper_Space0`) is the reverse of their tab order.
fn document_with_reversed_tab_order() -> acadrust::CadDocument {
    let mut doc = acadrust::CadDocument::new();
    // A fresh document has one paper layout (`Layout1`, block `*Paper_Space`).
    doc.add_layout("Layout2")
        .expect("adding a second paper layout succeeds");
    add_sheet_content(&mut doc, "Layout1", (90.0, 40.0), 50.0);
    add_sheet_content(&mut doc, "Layout2", (10.0, 10.0), 5.0);
    // Display order: Layout2 first even though its block record was created last.
    set_tab_order(&mut doc, "Layout1", 2);
    set_tab_order(&mut doc, "Layout2", 1);
    doc
}

#[test]
fn layouts_follow_tab_order_and_carry_display_names() {
    let doc = document_with_reversed_tab_order();
    let req = request(Vec::new());
    let importer = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let drawing = importer.run().expect("in-memory document imports");

    let layouts: Vec<_> = drawing.database.layouts().collect();
    assert_eq!(
        layouts
            .iter()
            .map(|layout| (layout.id, layout.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(LayoutId(1), "Layout2"), (LayoutId(2), "Layout1")],
        "ids follow the LAYOUT tab order and names are the display names"
    );

    // The content viewport and paper entity are associated through the block
    // record, so the reversed display order does not move them between sheets.
    let second = drawing
        .database
        .layout(LayoutId(1))
        .expect("Layout2 has a row");
    assert_eq!(second.viewports.len(), 1);
    assert_eq!(
        second.viewports[0].clip,
        vec![
            Point3 {
                x: 0.0,
                y: 5.0,
                z: 0.0
            },
            Point3 {
                x: 20.0,
                y: 5.0,
                z: 0.0
            },
            Point3 {
                x: 20.0,
                y: 15.0,
                z: 0.0
            },
            Point3 {
                x: 0.0,
                y: 15.0,
                z: 0.0
            },
        ]
    );
    let first = drawing
        .database
        .layout(LayoutId(2))
        .expect("Layout1 has a row");
    assert_eq!(first.viewports.len(), 1);
    assert_eq!(first.viewports[0].clip[0].x, 80.0);
    assert_eq!(first.viewports[0].clip[0].y, 35.0);

    // Layout entities include the imported viewport entities too; pin the paper
    // Line to the layout the block record belongs to.
    let line_starts = |layout: LayoutId| {
        drawing
            .database
            .layout_entities(layout)
            .iter()
            .filter_map(|entity| match &entity.geometry {
                SemanticGeometry::Line { start, .. } => Some(start.x),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(line_starts(LayoutId(1)), vec![5.0]);
    assert_eq!(line_starts(LayoutId(2)), vec![50.0]);
}

#[test]
fn a_paper_block_without_a_layout_object_is_appended_with_its_block_name() {
    let mut doc = acadrust::CadDocument::new();
    doc.add_layout("Layout2")
        .expect("adding a second paper layout succeeds");
    // Remove Layout2's object, leaving its `*Paper_Space0` block record (and
    // its entities) behind, as some older writers do.
    let second = layout_handle(&doc, "Layout2");
    doc.objects.remove(&second);
    add_sheet_content(&mut doc, "Layout1", (90.0, 40.0), 50.0);

    let req = request(Vec::new());
    let importer = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let drawing = importer.run().expect("in-memory document imports");

    let layouts: Vec<_> = drawing
        .database
        .layouts()
        .map(|layout| (layout.id, layout.name.as_str()))
        .collect();
    assert_eq!(layouts.len(), 2);
    assert_eq!(layouts[0], (LayoutId(1), "Layout1"));
    // Fallback keeps the block name: no display name exists without an object.
    assert!(
        layouts[1].1.eq_ignore_ascii_case("*Paper_Space0"),
        "{layouts:?}"
    );
    assert_eq!(layouts[1].0, LayoutId(2));
}
