//! `XCLIP` persistence: reading a block reference's clip boundary from the
//! `ACAD_FILTER`/`SPATIAL_FILTER` object chain, and what the exporter does with
//! a stored clip.
//!
//! Every DXF here is produced by the real acadrust writer (or written as the
//! literal group-code layout ACAD emits) and read back through the real
//! acadrust reader before this importer sees it, so the evidence covers the
//! parse → `BlockClip` path rather than a hand-constructed object graph.

use super::*;
use std::sync::Arc;

fn request(bytes: Vec<u8>) -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

// ---- fixtures ----

/// A document with one block (`name`) holding a single LINE, one model-space
/// INSERT of it at `insert_point`, and — when `boundary` is given — the
/// `ACAD_FILTER/SPATIAL_FILTER` chain ACAD attaches to a clipped INSERT.
///
/// `boundary` points are **WCS** boundary points, exactly as the format stores
/// them, and `inverse_block` is the WCS → block-definition transform ACAD
/// writes alongside them.
fn clipped_insert_document(
    name: &str,
    insert_point: [f64; 2],
    boundary: Option<(&[[f64; 2]], bool)>,
) -> acadrust::CadDocument {
    use acadrust::objects::{Dictionary, ObjectType, SpatialFilter};
    use acadrust::tables::BlockRecord;
    use acadrust::types::{Handle, Matrix4, Vector2, Vector3};

    let mut doc = acadrust::CadDocument::new();

    let block_handle = doc.allocate_handle();
    let block_entity_handle = doc.allocate_handle();
    let block_end_handle = doc.allocate_handle();
    let mut record = BlockRecord::new(name);
    record.handle = block_handle;
    record.block_entity_handle = block_entity_handle;
    record.block_end_handle = block_end_handle;
    record.base_point = Vector3::ZERO;
    doc.block_records.add_or_replace(record);

    // One member line so the block is non-empty and reachable.
    let member_handle = doc.allocate_handle();
    let mut line = acadrust::entities::Line::new();
    line.common.handle = member_handle;
    line.common.owner_handle = block_handle;
    line.start = Vector3::new(0.0, 0.0, 0.0);
    line.end = Vector3::new(1.0, 1.0, 0.0);
    doc.add_entity(EntityType::Line(line)).expect("add member");

    let insert_handle = doc.allocate_handle();
    let mut insert =
        acadrust::entities::Insert::new(name, Vector3::new(insert_point[0], insert_point[1], 0.0));
    insert.common.handle = insert_handle;

    if let Some((points, display_enabled)) = boundary {
        let xdict_handle = doc.allocate_handle();
        let filter_dict_handle = doc.allocate_handle();
        let spatial_handle = doc.allocate_handle();

        let mut xdict = Dictionary::new();
        xdict.handle = xdict_handle;
        xdict.owner = insert_handle;
        xdict.hard_owner = false;
        xdict.add_entry("ACAD_FILTER", filter_dict_handle);

        let mut filter_dict = Dictionary::new();
        filter_dict.handle = filter_dict_handle;
        filter_dict.owner = xdict_handle;
        filter_dict.add_entry("SPATIAL", spatial_handle);

        let mut filter = SpatialFilter::new();
        filter.handle = spatial_handle;
        filter.owner = filter_dict_handle;
        filter.boundary_points = points.iter().map(|p| Vector2::new(p[0], p[1])).collect();
        filter.display_enabled = display_enabled;
        // WCS → block definition space: subtract the insert point.
        filter.inverse_block_transform = Matrix4 {
            m: [
                [1.0, 0.0, 0.0, -insert_point[0]],
                [0.0, 1.0, 0.0, -insert_point[1]],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };

        doc.objects
            .insert(xdict_handle, ObjectType::Dictionary(xdict));
        doc.objects
            .insert(filter_dict_handle, ObjectType::Dictionary(filter_dict));
        doc.objects
            .insert(spatial_handle, ObjectType::SpatialFilter(filter));
        insert.common.xdictionary_handle = Some(Handle::new(xdict_handle.value()));
    }

    doc.add_entity(EntityType::Insert(insert))
        .expect("add insert");
    doc
}

/// Import one acadrust document through the real DXF writer + reader.
fn import_document(doc: &acadrust::CadDocument) -> ImportedDrawing {
    let bytes = acadrust::DxfWriter::new(doc)
        .write_to_vec()
        .expect("the fixture document writes");
    import_bytes(bytes)
}

fn import_bytes(bytes: Vec<u8>) -> ImportedDrawing {
    AcadrustImporter::default()
        .import(&request(bytes), &|| false)
        .expect("the fixture imports")
}

/// The clip stored on the model-space INSERT the fixture creates.
fn model_insert_clip(drawing: &ImportedDrawing) -> Option<BlockClip> {
    drawing
        .database
        .entities()
        .filter(|entity| entity.space == SpaceId::Model)
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Insert { .. }))
        .and_then(|entity| drawing.database.entity_render_attributes(entity.id).clip)
}

/// Whether any imported entity carries a stored clip at all.
fn any_stored_clip(drawing: &ImportedDrawing) -> bool {
    drawing.database.entities().any(|entity| {
        drawing
            .database
            .entity_render_attributes(entity.id)
            .clip
            .is_some()
    })
}

fn diagnostic_codes(drawing: &ImportedDrawing) -> Vec<String> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|d| d.code.clone())
        .collect()
}

// ---- import ----

#[test]
fn xclip_boundary_is_read_into_block_local_space() {
    // INSERT at (10, 20); ACAD stores the boundary in WCS and the inverse block
    // transform carries the -10/-20 translation.
    let doc = clipped_insert_document(
        "CLIPPED",
        [10.0, 20.0],
        Some((&[[8.0, 18.0], [12.0, 22.0]], true)),
    );
    let drawing = import_document(&doc);
    let clip = model_insert_clip(&drawing).expect("the clipped INSERT carries a boundary");

    assert_eq!(
        clip.vertices,
        vec![[-2.0, -2.0], [2.0, 2.0]],
        "the ring must be in the block definition's local space"
    );
    assert!(
        clip.is_rect(),
        "two boundary points are the rectangle's opposite corners"
    );
    assert!(clip.visible, "group 71 = 1 is the enabled clip");
    assert!(clip.inside, "the source model carries no clip-mode field");
    assert!(clip.is_active());
    let codes = diagnostic_codes(&drawing);
    assert!(
        codes.iter().any(|c| c == "import.xclip_mode_unmodelled"),
        "the model's missing clip-mode field is declared: {codes:?}"
    );
}

#[test]
fn xclip_polygon_boundary_keeps_every_vertex() {
    let doc = clipped_insert_document(
        "CLIPPED",
        [100.0, 100.0],
        Some((&[[90.0, 90.0], [110.0, 95.0], [105.0, 110.0]], true)),
    );
    let drawing = import_document(&doc);
    let clip = model_insert_clip(&drawing).expect("the clipped INSERT carries a boundary");
    assert_eq!(
        clip.vertices,
        vec![[-10.0, -10.0], [10.0, -5.0], [5.0, 10.0]]
    );
    assert!(!clip.is_rect(), "three points are a polygon");
}

#[test]
fn xclip_switched_off_keeps_the_boundary_but_does_not_clip() {
    let doc = clipped_insert_document(
        "CLIPPED",
        [10.0, 20.0],
        Some((&[[8.0, 18.0], [12.0, 22.0]], false)),
    );
    let drawing = import_document(&doc);
    let clip = model_insert_clip(&drawing).expect("XCLIP OFF still stores the boundary");
    assert!(!clip.visible, "group 71 = 0");
    assert!(
        !clip.is_active(),
        "a stored but switched-off clip cuts nothing"
    );
    assert_eq!(clip.vertices, vec![[-2.0, -2.0], [2.0, 2.0]]);
}

#[test]
fn unclipped_insert_imports_unclipped() {
    let doc = clipped_insert_document("PLAIN", [10.0, 20.0], None);
    let drawing = import_document(&doc);
    assert_eq!(model_insert_clip(&drawing), None);
    assert!(!any_stored_clip(&drawing));
    assert!(
        !diagnostic_codes(&drawing)
            .iter()
            .any(|code| code.starts_with("import.xclip_")),
        "an unclipped reference reports nothing: {:?}",
        diagnostic_codes(&drawing)
    );
}

#[test]
fn xclip_with_a_single_boundary_point_is_refused_by_name() {
    let doc = clipped_insert_document("CLIPPED", [10.0, 20.0], Some((&[[8.0, 18.0]], true)));
    let drawing = import_document(&doc);
    assert_eq!(
        model_insert_clip(&drawing),
        None,
        "a one-point boundary is no region"
    );
    assert!(
        diagnostic_codes(&drawing)
            .iter()
            .any(|code| code == "import.xclip_boundary_arity"),
        "{:?}",
        diagnostic_codes(&drawing)
    );
}

/// The literal group-code layout ACAD writes, not a writer-produced document:
/// the extension dictionary is a `102 {ACAD_XDICTIONARY` group on the INSERT,
/// the two dictionaries use `3`/`350` entries, and the filter carries the
/// point count (`70`), planes (`72`/`73`), and the two 12-double transforms.
#[test]
fn acad_written_group_layout_is_read() {
    let dxf = "\
0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n0\nENDSEC\n\
0\nSECTION\n2\nBLOCKS\n\
0\nBLOCK\n5\n20\n330\n1F\n8\n0\n2\nXBLK\n70\n0\n10\n0.0\n20\n0.0\n30\n0.0\n3\nXBLK\n\
0\nLINE\n5\n21\n330\n20\n8\n0\n10\n0.0\n20\n0.0\n30\n0.0\n11\n1.0\n21\n1.0\n31\n0.0\n\
0\nENDBLK\n5\n22\n330\n20\n8\n0\n\
0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nINSERT\n5\n30\n330\n1F\n8\n0\n2\nXBLK\n10\n10.0\n20\n20.0\n30\n0.0\n\
102\n{ACAD_XDICTIONARY\n360\n40\n102\n}\n\
0\nENDSEC\n\
0\nSECTION\n2\nOBJECTS\n\
0\nDICTIONARY\n5\n40\n330\n30\n100\nAcDbDictionary\n280\n0\n281\n1\n3\nACAD_FILTER\n350\n41\n\
0\nDICTIONARY\n5\n41\n330\n40\n100\nAcDbDictionary\n280\n0\n281\n1\n3\nSPATIAL\n350\n42\n\
0\nSPATIAL_FILTER\n5\n42\n330\n41\n100\nAcDbFilter\n100\nAcDbSpatialFilter\n\
70\n4\n10\n8.0\n20\n18.0\n10\n12.0\n20\n18.0\n10\n12.0\n20\n22.0\n10\n8.0\n20\n22.0\n\
210\n0.0\n220\n0.0\n230\n1.0\n11\n0.0\n21\n0.0\n31\n0.0\n71\n1\n72\n0\n73\n0\n\
40\n1.0\n40\n0.0\n40\n0.0\n40\n-10.0\n40\n0.0\n40\n1.0\n40\n0.0\n40\n-20.0\n\
40\n0.0\n40\n0.0\n40\n1.0\n40\n0.0\n\
40\n1.0\n40\n0.0\n40\n0.0\n40\n10.0\n40\n0.0\n40\n1.0\n40\n0.0\n40\n20.0\n\
40\n0.0\n40\n0.0\n40\n1.0\n40\n0.0\n\
0\nENDSEC\n0\nEOF\n";
    let drawing = import_bytes(dxf.as_bytes().to_vec());
    let clip = model_insert_clip(&drawing).expect("the ACAD layout is read");
    assert_eq!(
        clip.vertices,
        vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]]
    );
    assert!(clip.visible);
    assert!(clip.is_active());
}

#[test]
fn nested_block_reference_clip_is_stored_on_the_inner_insert() {
    use acadrust::objects::{Dictionary, ObjectType, SpatialFilter};
    use acadrust::tables::BlockRecord;
    use acadrust::types::{Matrix4, Vector2, Vector3};

    let mut doc = acadrust::CadDocument::new();
    // INNER: one line. OUTER: an INSERT of INNER carrying the clip.
    for name in ["INNER", "OUTER"] {
        let handle = doc.allocate_handle();
        let entity_handle = doc.allocate_handle();
        let end_handle = doc.allocate_handle();
        let mut record = BlockRecord::new(name);
        record.handle = handle;
        record.block_entity_handle = entity_handle;
        record.block_end_handle = end_handle;
        record.base_point = Vector3::ZERO;
        doc.block_records.add_or_replace(record);
    }
    let inner_block = doc
        .block_records
        .iter()
        .find(|b| b.name == "INNER")
        .expect("INNER")
        .handle;
    let outer_block = doc
        .block_records
        .iter()
        .find(|b| b.name == "OUTER")
        .expect("OUTER")
        .handle;

    let mut inner_line = acadrust::entities::Line::new();
    inner_line.common.handle = doc.allocate_handle();
    inner_line.common.owner_handle = inner_block;
    inner_line.start = Vector3::ZERO;
    inner_line.end = Vector3::new(1.0, 0.0, 0.0);
    doc.add_entity(EntityType::Line(inner_line))
        .expect("add INNER line");

    // The clipped reference lives inside OUTER, at (50, 0) in block space.
    let nested_handle = doc.allocate_handle();
    let mut nested = acadrust::entities::Insert::new("INNER", Vector3::new(50.0, 0.0, 0.0));
    nested.common.handle = nested_handle;
    nested.common.owner_handle = outer_block;

    let xdict_handle = doc.allocate_handle();
    let filter_dict_handle = doc.allocate_handle();
    let spatial_handle = doc.allocate_handle();
    let mut xdict = Dictionary::new();
    xdict.handle = xdict_handle;
    xdict.owner = nested_handle;
    xdict.add_entry("ACAD_FILTER", filter_dict_handle);
    let mut filter_dict = Dictionary::new();
    filter_dict.handle = filter_dict_handle;
    filter_dict.owner = xdict_handle;
    filter_dict.add_entry("SPATIAL", spatial_handle);
    let mut filter = SpatialFilter::new();
    filter.handle = spatial_handle;
    filter.owner = filter_dict_handle;
    filter.boundary_points = vec![Vector2::new(48.0, -2.0), Vector2::new(52.0, 2.0)];
    filter.inverse_block_transform = Matrix4 {
        m: [
            [1.0, 0.0, 0.0, -50.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    doc.objects
        .insert(xdict_handle, ObjectType::Dictionary(xdict));
    doc.objects
        .insert(filter_dict_handle, ObjectType::Dictionary(filter_dict));
    doc.objects
        .insert(spatial_handle, ObjectType::SpatialFilter(filter));
    nested.common.xdictionary_handle = Some(xdict_handle);
    doc.add_entity(EntityType::Insert(nested))
        .expect("add nested insert");

    // Model space references OUTER twice, so the nested clip is reached through
    // an ordinary block expansion.
    for x in [0.0, 200.0] {
        let mut outer = acadrust::entities::Insert::new("OUTER", Vector3::new(x, 0.0, 0.0));
        outer.common.handle = doc.allocate_handle();
        doc.add_entity(EntityType::Insert(outer))
            .expect("add outer insert");
    }

    let drawing = import_document(&doc);
    let nested_clips: Vec<BlockClip> = drawing
        .database
        .entities()
        .filter(|entity| matches!(entity.space, SpaceId::Block(_)))
        .filter(|entity| matches!(entity.geometry, SemanticGeometry::Insert { .. }))
        .filter_map(|entity| drawing.database.entity_render_attributes(entity.id).clip)
        .collect();
    assert_eq!(
        nested_clips.len(),
        1,
        "the block definition holds one clipped reference"
    );
    assert_eq!(
        nested_clips[0].vertices,
        vec![[-2.0, -2.0], [2.0, 2.0]],
        "the nested boundary is in INNER's local space"
    );
    assert!(nested_clips[0].is_active());
}

// ---- export ----

#[test]
fn export_writes_the_clip_chain_so_an_imported_clip_round_trips() {
    // This test used to assert the loss: the exporter wrote no INSERT at all, so
    // a stored XCLIP had nowhere to go (`export.xclip_clip_dropped`). Block
    // references are now written as real INSERT records, and a representable
    // clip is written as its ACAD_FILTER/SPATIAL/SPATIAL_FILTER chain.
    let doc = clipped_insert_document(
        "CLIPPED",
        [10.0, 20.0],
        Some((&[[8.0, 18.0], [12.0, 22.0]], true)),
    );
    let imported = import_document(&doc);
    let before = model_insert_clip(&imported).expect("precondition: the source clip is read");
    assert_eq!(before.vertices, vec![[-2.0, -2.0], [2.0, 2.0]]);

    let (bytes, report) = crate::export::export(&crate::export::ExportRequest {
        database: &imported.database,
        document_id: DocumentId(1),
        space: crate::export::ExportSpace::Model,
        format: crate::export::ExportFormat::DxfText,
        limits: crate::export::ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");

    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.xclip_clip_dropped"),
        "the clip is written, so no clip loss may be reported: {:?}",
        report.entries
    );
    let text = std::str::from_utf8(&bytes).expect("ASCII DXF is valid UTF-8");
    for marker in ["INSERT", "SPATIAL_FILTER", "ACAD_FILTER", "SPATIAL"] {
        assert!(
            text.contains(marker),
            "the chain is written: missing {marker}"
        );
    }

    // The boundary comes back in the referenced block's local space, with the
    // enabled state it was stored with.
    let reimported = import_bytes(bytes);
    let after = model_insert_clip(&reimported).expect("the exported clip is read back");
    assert_eq!(after.vertices, before.vertices);
    assert_eq!(after.visible, before.visible);
    assert_eq!(after.inside, before.inside);
    assert!(after.is_active());
}

#[test]
fn export_writes_a_switched_off_clip_as_a_disabled_boundary() {
    // XCLIP OFF stores a boundary that is not applied: group 71 = 0. It must
    // round-trip as a boundary that is present but inactive, never as "no clip".
    let doc = clipped_insert_document(
        "CLIPPED",
        [10.0, 20.0],
        Some((&[[8.0, 18.0], [12.0, 22.0]], false)),
    );
    let imported = import_document(&doc);
    let (bytes, _) = crate::export::export(&crate::export::ExportRequest {
        database: &imported.database,
        document_id: DocumentId(1),
        space: crate::export::ExportSpace::Model,
        format: crate::export::ExportFormat::DxfText,
        limits: crate::export::ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");

    let reimported = import_bytes(bytes);
    let clip = model_insert_clip(&reimported).expect("the boundary is still stored");
    assert!(!clip.visible, "group 71 = 0 round-trips");
    assert_eq!(clip.vertices, vec![[-2.0, -2.0], [2.0, 2.0]]);
    assert!(!clip.is_active());
}
