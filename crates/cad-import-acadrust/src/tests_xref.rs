//! External-reference (XREF) resolution and merge-at-import contracts.
//!
//! Every referenced file here is synthetic: host documents are built as
//! `acadrust::CadDocument` values and referenced files are written with
//! acadrust's own DXF writer. No fixture file is added and no network/real
//! filesystem is required; the loader is a recording in-memory
//! [`XrefFileSource`].

use super::*;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cad_resources::{ResourceLimits, XrefFileSource, XrefSourceError};

const DIR: &str = "/drawings";

// ---------------------------------------------------------------------------
// In-memory loader
// ---------------------------------------------------------------------------

struct MemorySource {
    files: HashMap<PathBuf, Arc<[u8]>>,
    browser: bool,
    fail: bool,
    reads: Mutex<Vec<PathBuf>>,
}

impl MemorySource {
    fn new(files: &[(&str, Arc<[u8]>)]) -> Self {
        Self::from_owned(
            files
                .iter()
                .map(|(name, bytes)| ((*name).to_string(), Arc::clone(bytes)))
                .collect(),
        )
    }

    fn from_owned(files: Vec<(String, Arc<[u8]>)>) -> Self {
        let files = files
            .into_iter()
            .map(|(name, bytes)| (Path::new(DIR).join(name), bytes))
            .collect();
        Self {
            files,
            browser: false,
            fail: false,
            reads: Mutex::new(Vec::new()),
        }
    }

    fn browser() -> Self {
        Self {
            files: HashMap::new(),
            browser: true,
            fail: false,
            reads: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            files: HashMap::new(),
            browser: false,
            fail: true,
            reads: Mutex::new(Vec::new()),
        }
    }

    fn read_count(&self) -> usize {
        self.reads.lock().unwrap().len()
    }
}

impl XrefFileSource for MemorySource {
    fn read(&self, path: &Path) -> Result<Option<Arc<[u8]>>, XrefSourceError> {
        self.reads.lock().unwrap().push(path.to_path_buf());
        if self.browser {
            return Ok(None);
        }
        if self.fail {
            return Err(XrefSourceError::Missing(path.display().to_string()));
        }
        self.files.get(path).cloned().map(Some).ok_or_else(|| {
            XrefSourceError::Missing(format!("missing in-memory xref: {}", path.display()))
        })
    }
}

fn options(source: Arc<MemorySource>) -> XrefOptions {
    XrefOptions::new(source, DIR)
}

// ---------------------------------------------------------------------------
// Synthetic acadrust documents
// ---------------------------------------------------------------------------

fn dxf_bytes(document: &acadrust::CadDocument) -> Arc<[u8]> {
    let bytes = acadrust::DxfWriter::new(document)
        .write_to_vec()
        .expect("synthetic DXF writes");
    Arc::from(bytes.into_boxed_slice())
}

fn add_layer(document: &mut acadrust::CadDocument, name: &str, color: acadrust::Color) {
    if document.layers.get(name).is_some() {
        return;
    }
    let mut layer = acadrust::tables::Layer::new(name);
    layer.handle = document.allocate_handle();
    layer.color = color;
    document.layers.add(layer).expect("add layer");
}

#[derive(Clone, Copy, Default)]
struct XrefSpec<'a> {
    path: &'a str,
    overlay: bool,
    unloaded: bool,
}

fn add_xref_block(
    document: &mut acadrust::CadDocument,
    name: &str,
    spec: XrefSpec<'_>,
) -> acadrust::Handle {
    let handle = document.allocate_handle();
    let mut record = acadrust::tables::BlockRecord::new(name);
    record.handle = handle;
    record.block_entity_handle = document.allocate_handle();
    record.block_end_handle = document.allocate_handle();
    record.flags.is_xref = true;
    record.flags.is_xref_overlay = spec.overlay;
    record.flags.is_xref_unloaded = spec.unloaded;
    record.xref_path = spec.path.to_string();
    document
        .block_records
        .add(record)
        .expect("add xref block record");
    handle
}

fn add_block(document: &mut acadrust::CadDocument, name: &str) -> acadrust::Handle {
    let handle = document.allocate_handle();
    let mut record = acadrust::tables::BlockRecord::new(name);
    record.handle = handle;
    record.block_entity_handle = document.allocate_handle();
    record.block_end_handle = document.allocate_handle();
    document
        .block_records
        .add(record)
        .expect("add block record");
    handle
}

fn add_block_line(
    document: &mut acadrust::CadDocument,
    block: acadrust::Handle,
    layer: &str,
    y: f64,
) -> acadrust::Handle {
    let mut line = acadrust::entities::Line::new();
    line.start = acadrust::types::Vector3::new(0.0, y, 0.0);
    line.end = acadrust::types::Vector3::new(1.0, y, 0.0);
    line.common.layer = layer.to_string();
    line.common.owner_handle = block;
    document
        .add_entity(EntityType::Line(line))
        .expect("add block line")
}

fn add_model_insert(
    document: &mut acadrust::CadDocument,
    block: &str,
    at: acadrust::types::Vector3,
) -> acadrust::Handle {
    let insert = acadrust::entities::Insert::new(block, at);
    document
        .add_entity(EntityType::Insert(insert))
        .expect("add model insert")
}

fn add_model_line(
    document: &mut acadrust::CadDocument,
    layer: &str,
    start: acadrust::types::Vector3,
    end: acadrust::types::Vector3,
) {
    let mut line = acadrust::entities::Line::new();
    line.start = start;
    line.end = end;
    line.common.layer = layer.to_string();
    document
        .add_entity(EntityType::Line(line))
        .expect("add line");
}

fn run_import(
    document: &acadrust::CadDocument,
    xref: Option<Arc<XrefOptions>>,
) -> CadResult<ImportedDrawing> {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(Vec::<u8>::new().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref,
    };
    ImporterBuilder::new(
        &request,
        document,
        ReadStats::default(),
        compute_identity(&[]),
    )
    .run()
}

fn reference<'a>(drawing: &'a ImportedDrawing, name: &str) -> &'a ExternalReference {
    drawing
        .database
        .external_reference(name)
        .unwrap_or_else(|| panic!("external reference '{name}' is recorded"))
}

fn has_diagnostic(drawing: &ImportedDrawing, code: &str) -> bool {
    drawing.report.diagnostics.iter().any(|d| d.code == code)
}

fn block_named<'a>(drawing: &'a ImportedDrawing, name: &str) -> &'a BlockDefinition {
    drawing
        .database
        .block_by_name(name)
        .unwrap_or_else(|| panic!("block '{name}' exists"))
}

fn entity(drawing: &ImportedDrawing, id: EntityId) -> &DbEntity {
    drawing.database.entity(id).expect("entity exists")
}

// ---------------------------------------------------------------------------
// Host documents
// ---------------------------------------------------------------------------

fn host_with_xref(
    name: &str,
    spec: XrefSpec<'_>,
    insert_at: acadrust::types::Vector3,
) -> acadrust::CadDocument {
    let mut document = acadrust::CadDocument::new();
    add_xref_block(&mut document, name, spec);
    add_model_insert(&mut document, name, insert_at);
    document
}

fn reference_line_doc(layer: &str, y: f64) -> acadrust::CadDocument {
    let mut document = acadrust::CadDocument::new();
    add_layer(&mut document, layer, acadrust::Color::WHITE);
    add_model_line(
        &mut document,
        layer,
        acadrust::types::Vector3::new(0.0, y, 0.0),
        acadrust::types::Vector3::new(1.0, y, 0.0),
    );
    document
}

// ---------------------------------------------------------------------------
// Loader/path/depth/budget contract
// ---------------------------------------------------------------------------

#[test]
fn browser_source_reports_unavailable_not_empty_success() {
    let document = host_with_xref(
        "BASE",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let source = Arc::new(MemorySource::browser());
    let drawing = run_import(&document, Some(Arc::new(options(Arc::clone(&source)))))
        .expect("import succeeds");
    assert_eq!(
        reference(&drawing, "BASE").status,
        ExternalReferenceStatus::BrowserUnavailable
    );
    assert!(has_diagnostic(&drawing, "import.xref_unavailable"));
    assert_eq!(source.read_count(), 1);
}

#[test]
fn missing_file_is_missing_and_never_a_silent_empty_block() {
    let document = host_with_xref(
        "BASE",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let source = Arc::new(MemorySource::failing());
    let drawing = run_import(&document, Some(Arc::new(options(Arc::clone(&source)))))
        .expect("import succeeds");
    assert_eq!(
        reference(&drawing, "BASE").status,
        ExternalReferenceStatus::Missing
    );
    assert!(has_diagnostic(&drawing, "import.xref_unreadable"));
    assert!(!matches!(
        drawing.report.completeness,
        Completeness::Complete
    ));

    // The report and capability must reflect the terminal status, not the
    // pre-resolution classification (the pending placeholder looked `Complete`
    // when `read_entities` ran).
    assert!(has_diagnostic(&drawing, "import.xref_insert_missing"));
    let capability = drawing
        .report
        .capabilities
        .iter()
        .find(|capability| capability.type_key == "AcDbBlockReference")
        .expect("the INSERT capability row exists");
    assert_ne!(
        capability.semantic,
        SupportStatus::Verified,
        "{capability:?}"
    );
    assert_ne!(capability.render, SupportStatus::Verified, "{capability:?}");
}

#[test]
fn run_level_missing_xref_weakens_the_insert_and_report() {
    // Production order: blocks, entities (INSERT classified as Complete while
    // the xref is still pending), then resolution. The post-resolution pass must
    // expose the real loss.
    let document = host_with_xref(
        "BASE",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let source = Arc::new(MemorySource::failing());
    let drawing = run_import(&document, Some(Arc::new(options(source)))).expect("import succeeds");

    let block = block_named(&drawing, "BASE");
    assert!(block.entities.is_empty(), "{block:?}");
    let insert = drawing
        .database
        .entities()
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Insert { .. }))
        .expect("the host INSERT is present");
    assert!(matches!(insert.space, SpaceId::Model));
    assert!(has_diagnostic(&drawing, "import.xref_insert_missing"));

    let capability = drawing
        .report
        .capabilities
        .iter()
        .find(|capability| capability.type_key == "AcDbBlockReference")
        .expect("the INSERT capability row exists");
    assert_ne!(
        capability.semantic,
        SupportStatus::Verified,
        "{capability:?}"
    );
    assert_ne!(
        capability.measure,
        SupportStatus::Verified,
        "{capability:?}"
    );
    assert!(!matches!(
        drawing.report.completeness,
        Completeness::Complete | Completeness::Unverified
    ));
}

#[test]
fn loader_refusal_is_refused_path_not_missing() {
    struct RefusingSource;
    impl XrefFileSource for RefusingSource {
        fn read(&self, _path: &Path) -> Result<Option<Arc<[u8]>>, XrefSourceError> {
            Err(XrefSourceError::Refused(
                "canonical path outside allowed roots".into(),
            ))
        }
    }
    let document = host_with_xref(
        "BASE",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let drawing = run_import(
        &document,
        Some(Arc::new(XrefOptions::new(Arc::new(RefusingSource), DIR))),
    )
    .expect("import succeeds");
    assert_eq!(
        reference(&drawing, "BASE").status,
        ExternalReferenceStatus::RefusedPath
    );
    assert!(has_diagnostic(&drawing, "import.xref_refused"));
}

#[test]
fn unsafe_absolute_path_is_refused_without_reading() {
    let document = host_with_xref(
        "ESCAPE",
        XrefSpec {
            path: "/etc/passwd",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let source = Arc::new(MemorySource::new(&[]));
    let drawing = run_import(&document, Some(Arc::new(options(Arc::clone(&source)))))
        .expect("import succeeds");
    assert_eq!(
        reference(&drawing, "ESCAPE").status,
        ExternalReferenceStatus::RefusedPath
    );
    assert!(has_diagnostic(&drawing, "import.xref_refused"));
    assert_eq!(source.read_count(), 0);
}

#[test]
fn unloaded_attachment_is_never_read() {
    let document = host_with_xref(
        "BASE",
        XrefSpec {
            path: "base.dxf",
            unloaded: true,
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let source = Arc::new(MemorySource::new(&[(
        "base.dxf",
        dxf_bytes(&reference_line_doc("0", 0.0)),
    )]));
    let drawing = run_import(&document, Some(Arc::new(options(Arc::clone(&source)))))
        .expect("import succeeds");
    assert_eq!(
        reference(&drawing, "BASE").status,
        ExternalReferenceStatus::Unloaded
    );
    assert!(has_diagnostic(&drawing, "import.xref_unloaded"));
    assert_eq!(source.read_count(), 0);
}

#[test]
fn depth_nine_is_refused_before_reading() {
    // Host -> level0 -> ... -> level8. The attachment inside level8 is at depth
    // 9 and must be refused before its file is read (level9 is not supplied).
    let mut files = Vec::new();
    for level in 0..=8 {
        let mut document = acadrust::CadDocument::new();
        add_xref_block(
            &mut document,
            "next",
            XrefSpec {
                path: &format!("level{}.dxf", level + 1),
                ..XrefSpec::default()
            },
        );
        add_model_insert(&mut document, "next", acadrust::types::Vector3::ZERO);
        files.push((format!("level{level}.dxf"), dxf_bytes(&document)));
    }
    let source = Arc::new(MemorySource::from_owned(files));
    let document = host_with_xref(
        "BASE",
        XrefSpec {
            path: "level0.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let options =
        options(Arc::clone(&source)).with_max_depth(ResourceLimits::default().max_xref_depth);
    let drawing = run_import(&document, Some(Arc::new(options))).expect("import succeeds");

    let depth = drawing
        .database
        .external_references()
        .find(|reference| reference.status == ExternalReferenceStatus::DepthExceeded)
        .expect("the depth-9 attachment is recorded");
    assert_eq!(depth.depth, 9, "{depth:?}");
    assert!(has_diagnostic(&drawing, "import.xref_depth"));
    // The refused file was never requested: only level0..level8 were read.
    assert_eq!(source.read_count(), 9);
}

#[test]
fn total_byte_budget_is_enforced_before_parse() {
    let document = host_with_xref(
        "BASE",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let bytes = dxf_bytes(&reference_line_doc("0", 0.0));
    let source = Arc::new(MemorySource::new(&[("base.dxf", bytes)]));
    // The five-field literal shape is part of the host contract; this keeps a
    // private session field from sneaking back into the public struct.
    let options = XrefOptions {
        source: source.clone(),
        drawing_dir: PathBuf::from(DIR),
        roots: Vec::new(),
        max_depth: ResourceLimits::default().max_xref_depth,
        total_bytes_budget: 16,
    };
    let drawing = run_import(&document, Some(Arc::new(options))).expect("import succeeds");
    assert_eq!(
        reference(&drawing, "BASE").status,
        ExternalReferenceStatus::Unsupported
    );
    assert!(has_diagnostic(&drawing, "import.xref_budget"));
    assert_eq!(source.read_count(), 1);
}

// ---------------------------------------------------------------------------
// Loaded merge
// ---------------------------------------------------------------------------

#[test]
fn loaded_xref_merges_model_members_and_namespaces_layers() {
    let document = host_with_xref(
        "XREF",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::new(10.0, 10.0, 0.0),
    );
    let source = Arc::new(MemorySource::new(&[(
        "base.dxf",
        dxf_bytes(&reference_line_doc("0", 0.0)),
    )]));
    let drawing = run_import(&document, Some(Arc::new(options(source)))).expect("import succeeds");

    let reference = reference(&drawing, "XREF");
    assert_eq!(reference.status, ExternalReferenceStatus::Loaded);
    assert!(reference.resolved_path.is_some());

    // Referenced model content becomes a member of the host xref block.
    let block = block_named(&drawing, "XREF");
    assert_eq!(block.entities.len(), 1, "{block:?}");
    let member = entity(&drawing, block.entities[0]);
    assert!(matches!(member.space, SpaceId::Block(id) if id == block.id));
    assert!(matches!(
        member.geometry,
        SemanticGeometry::Line { start, end }
            if (start.x).abs() < 1e-9 && (end.x - 1.0).abs() < 1e-9
    ));

    // The host INSERT expands the block at its insertion point, so the bounds
    // include the referenced line translated to x = 10.
    let (min, max) = drawing.database.bounds().expect("bounds");
    assert!((min.x - 10.0).abs() < 1e-6, "{min:?} {max:?}");

    // Xref-dependent layer row exists and carries the flag.
    let layer = drawing
        .database
        .layers()
        .find(|layer| layer.name == "XREF|0")
        .expect("namespaced xref layer");
    assert!(drawing.database.layer_attributes(layer.id).xref_dependent);
}

#[test]
fn empty_referenced_model_is_loaded_with_an_informational_diagnostic() {
    let document = host_with_xref(
        "EMPTY",
        XrefSpec {
            path: "empty.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    // A real drawing with a block definition but no model-space entity.
    let mut referenced = acadrust::CadDocument::new();
    let block = add_block(&mut referenced, "ONLY_BLOCK");
    add_block_line(&mut referenced, block, "0", 0.0);
    let source = Arc::new(MemorySource::new(&[("empty.dxf", dxf_bytes(&referenced))]));
    let drawing = run_import(&document, Some(Arc::new(options(source)))).expect("import succeeds");
    assert_eq!(
        reference(&drawing, "EMPTY").status,
        ExternalReferenceStatus::Loaded
    );
    assert!(has_diagnostic(&drawing, "import.xref_empty"));
    assert_eq!(block_named(&drawing, "EMPTY").entities.len(), 0);
}

#[test]
fn host_layer_override_wins_over_referenced_attributes() {
    let mut document = host_with_xref(
        "XREF",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    // A pre-existing xref-dependent host row with the target name and a red
    // colour. It must be reused unchanged (VISRETAIN-like host wins).
    add_layer(&mut document, "XREF|0", acadrust::Color::Index(1));
    document
        .layers
        .get_mut("XREF|0")
        .expect("host override layer")
        .flags
        .xref_dependent = true;

    let source = Arc::new(MemorySource::new(&[(
        "base.dxf",
        dxf_bytes(&reference_line_doc("0", 0.0)),
    )]));
    let drawing = run_import(&document, Some(Arc::new(options(source)))).expect("import succeeds");

    let overrides: Vec<_> = drawing
        .database
        .layers()
        .filter(|layer| layer.name == "XREF|0")
        .collect();
    assert_eq!(overrides.len(), 1, "the host row is reused, not duplicated");
    let attributes = drawing.database.layer_attributes(overrides[0].id);
    assert!(attributes.xref_dependent);
    assert_eq!(
        attributes.color.rgb,
        [255, 0, 0],
        "host colour is preserved"
    );
}

#[test]
fn non_xref_host_layer_with_the_target_name_is_reused_not_duplicated() {
    let mut document = host_with_xref(
        "XREF",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    // Same target name as the spliced referenced layer, but *not* marked
    // xref-dependent: the host row is still the override, never a duplicate.
    add_layer(&mut document, "XREF|0", acadrust::Color::Index(1));

    let source = Arc::new(MemorySource::new(&[(
        "base.dxf",
        dxf_bytes(&reference_line_doc("0", 0.0)),
    )]));
    let drawing = run_import(&document, Some(Arc::new(options(source)))).expect("import succeeds");

    let rows: Vec<_> = drawing
        .database
        .layers()
        .filter(|layer| layer.name == "XREF|0")
        .collect();
    assert_eq!(rows.len(), 1, "never two rows with the same name");
    assert_eq!(
        drawing.database.layer_attributes(rows[0].id).color.rgb,
        [255, 0, 0],
        "the host row attributes win"
    );
    assert!(
        has_diagnostic(&drawing, "import.xref_layer_collision"),
        "{:?}",
        drawing.report.diagnostics
    );
}

#[test]
fn insert_to_unloaded_xref_is_missing_with_a_stable_message() {
    let mut document = acadrust::CadDocument::new();
    add_xref_block(
        &mut document,
        "BASE",
        XrefSpec {
            path: "base.dxf",
            unloaded: true,
            ..XrefSpec::default()
        },
    );
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(Vec::<u8>::new().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: Some(Arc::new(options(Arc::new(MemorySource::new(&[]))))),
    };
    let mut builder = ImporterBuilder::new(
        &request,
        &document,
        ReadStats::default(),
        compute_identity(&[]),
    );
    builder.read_blocks().expect("read blocks");
    let insert = acadrust::entities::Insert::new("BASE", acadrust::types::Vector3::ZERO);
    let (_geometry, completeness) = builder.insert_semantics(&insert);
    match completeness {
        Completeness::Missing(reasons) => {
            assert!(
                reasons.iter().any(|reason| reason.contains("xref 'BASE'")),
                "{reasons:?}"
            );
            assert!(
                reasons.iter().any(|reason| reason.contains("unloaded")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[test]
fn insert_to_a_missing_xref_is_missing_after_resolution() {
    let mut document = acadrust::CadDocument::new();
    add_xref_block(
        &mut document,
        "BASE",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
    );
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(Vec::<u8>::new().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: Some(Arc::new(options(Arc::new(MemorySource::failing())))),
    };
    let mut builder = ImporterBuilder::new(
        &request,
        &document,
        ReadStats::default(),
        compute_identity(&[]),
    );
    builder.read_blocks().expect("read blocks");
    // Production order: entities are classified before xref resolution, so the
    // pending placeholder must not be treated as a missing reference then.
    builder.read_entities().expect("read entities");
    builder.resolve_xrefs().expect("resolve refs");
    let insert = acadrust::entities::Insert::new("BASE", acadrust::types::Vector3::ZERO);
    match builder.insert_semantics(&insert).1 {
        Completeness::Missing(reasons) => {
            assert!(
                reasons.iter().any(|reason| reason.contains("missing")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[test]
fn insert_to_an_xref_without_a_loader_is_missing() {
    let mut document = acadrust::CadDocument::new();
    add_xref_block(
        &mut document,
        "BASE",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
    );
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(Vec::<u8>::new().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let mut builder = ImporterBuilder::new(
        &request,
        &document,
        ReadStats::default(),
        compute_identity(&[]),
    );
    builder.read_blocks().expect("read blocks");
    let insert = acadrust::entities::Insert::new("BASE", acadrust::types::Vector3::ZERO);
    match builder.insert_semantics(&insert).1 {
        Completeness::Missing(reasons) => {
            assert!(
                reasons
                    .iter()
                    .any(|reason| reason.contains("browser-unavailable")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[path = "tests_xref_nested.rs"]
mod nested;
