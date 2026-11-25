//! Save-time collection of external resources and the recovery rule
//! (ADR-0040 D9/D10/D11/D13/D17).
//!
//! The collector is I/O around the authoritative database, and the archive is
//! carrier-owned, so these contracts drive a **fake** archive that captures the
//! snapshot the host asks it to encode. What is under test is exactly the host
//! half: which candidates are accepted, what lands in `cad-db` before the
//! snapshot, what the save reports, and that a missing/over-budget resource never
//! turns a save into a failure.
//!
//! The font files are synthetic bytes: the collector copies bytes and never parses
//! a face (shaping is `cad-platform`'s job), so a parseable font would add nothing.

use super::*;

use cad_db::{
    DbEntity, DbObject, DocumentSnapshot, DrawingDatabaseBuilder, Layer, Style, TableKind,
};
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, ObjectId, Revision, SemanticGeometry, SpaceId,
    StyleId, TextAlignH, TextAlignV, UnitContext,
};
use cad_platform::{
    ArchiveBytes, ArchiveError, ArchiveWrite, DocumentArchive, LoadedArchive, Persistence,
    RecoveryKey, RecoveryRecord, RecoverySnapshot,
};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const FONT_NAME: &str = "arial.ttf";
const MISSING_NAME: &str = "absent.ttf";
const FONT_BYTES: &[u8] = b"synthetic-font-bytes";

/// A scratch directory that is removed when the test drops it.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yacr-resources-{label}-{}-{}",
            std::process::id(),
            // One directory per test name, so parallel tests cannot collide.
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write one readable "font" into `dir` and return its path.
fn write_font(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("font file");
    path
}

/// A document with one TEXT entity whose style names `fonts`.
fn font_document(fonts: &[&str]) -> cad_db::DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(7));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("layer");
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: fonts.iter().map(|font| (*font).to_string()).collect(),
        })
        .expect("style");
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbText".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Text {
                text: "note".into(),
                position: p(0.0, 0.0),
                style: StyleId(0),
                height: 2.5,
                rotation: 0.0,
                font: Some(FONT_NAME.into()),
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
                wrap_width: None,
                width_factor: 1.0,
                line_spacing: cad_domain::TextLineSpacing::default(),
                mirrored: false,
            },
            draw_order: 0,
        })
        .expect("text");
    builder.finish().expect("database")
}

/// Install `drawing` as the authoritative document, exactly like a real open.
fn install(core: &HostCore, drawing: cad_db::DrawingDatabase) {
    let revision = drawing.revision();
    let snapshot = drawing
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            revision,
        )
        .expect("snapshot");
    core.controller
        .borrow_mut()
        .install_snapshot(
            &snapshot,
            cad_app::PersistenceState::imported(),
            // No extension: the save funnel's purpose rule then selects the
            // native archive rather than the lossy DXF export.
            Some("plan".into()),
        )
        .expect("install");
}

/// A core whose effective font directory is `fonts_dir` and whose images are
/// resolved from the drawing's own directory only.
fn resources_core(files: Rc<dyn PlatformFiles>, fonts_dir: Option<PathBuf>) -> HostCore {
    let platform = HostPlatform {
        files,
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let mut options = options();
    options.fonts_dir = fonts_dir;
    HostCore::new(options, platform, RenderSessionHandle::default()).expect("core")
}

/// A fake archive that captures every snapshot it was asked to encode.
///
/// `saved` mirrors a real store's save → reopen round trip (the next `read`
/// serves the last written snapshot back).
#[derive(Default)]
struct CapturingArchive {
    writes: Mutex<Vec<DocumentSnapshot>>,
    scopes: Mutex<Vec<cad_db::ChangeScope>>,
    saved: Mutex<Option<DocumentSnapshot>>,
}

impl CapturingArchive {
    fn writes(&self) -> Vec<DocumentSnapshot> {
        self.writes.lock().unwrap().clone()
    }

    fn scopes(&self) -> Vec<cad_db::ChangeScope> {
        self.scopes.lock().unwrap().clone()
    }
}

impl DocumentArchive for CapturingArchive {
    fn read(&self, _bytes: &[u8]) -> Result<LoadedArchive, ArchiveError> {
        match self.saved.lock().unwrap().clone() {
            Some(snapshot) => Ok(LoadedArchive {
                manifest: cad_db::ManifestSummary {
                    format: cad_db::FormatVersion {
                        read: 1,
                        write: 1,
                        minor: 0,
                    },
                    document_uuid: "doc-uuid-1".to_string(),
                    open_revision: Revision(0),
                    saved_revision: snapshot.revision,
                    foreign_entries: Vec::new(),
                },
                snapshot,
            }),
            None => Err(ArchiveError::new(
                "ymodel.invalid_document",
                "nothing written",
            )),
        }
    }

    fn write(&self, request: &ArchiveWrite<'_>) -> Result<ArchiveBytes, ArchiveError> {
        self.writes.lock().unwrap().push(request.snapshot.clone());
        self.scopes.lock().unwrap().push(request.dirty.clone());
        *self.saved.lock().unwrap() = Some(request.snapshot.clone());
        // Deterministic for one snapshot: the "bytes" only distinguish revisions.
        Ok(ArchiveBytes {
            bytes: format!("ARCHIVE-{}", request.snapshot.revision.0).into_bytes(),
            saved_revision: request.snapshot.revision,
        })
    }
}

/// A platform with a tiny in-memory disk (the save funnel re-reads an in-place
/// target) and a synchronous save target.
#[derive(Default)]
struct ResourceFiles {
    events: RefCell<Vec<FileEvent>>,
    disk: RefCell<BTreeMap<String, Vec<u8>>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
    source: RefCell<Option<FileTarget>>,
}

impl PlatformFiles for ResourceFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        let target = FileTarget {
            display_name: default_name.to_string(),
            opaque: format!("/tmp/{default_name}"),
        };
        self.events
            .borrow_mut()
            .push(FileEvent::SaveTarget { purpose, target });
        Ok(())
    }
    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        self.written
            .borrow_mut()
            .push((target.display_name.clone(), bytes.to_vec()));
        self.disk
            .borrow_mut()
            .insert(target.opaque.clone(), bytes.to_vec());
        Ok(())
    }
    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }
    fn read_target(&self, target: &FileTarget) -> Result<Vec<u8>, String> {
        self.disk
            .borrow()
            .get(&target.opaque)
            .cloned()
            .ok_or_else(|| "read-target: not found".to_string())
    }
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

/// A recovery store that only needs to accept writes.
#[derive(Default)]
struct QuietPersistence {
    saved: Mutex<Vec<Revision>>,
}

impl Persistence for QuietPersistence {
    fn save_recovery(&self, record: &RecoveryRecord, _bytes: &[u8]) -> CadResult<()> {
        self.saved.lock().unwrap().push(record.revision);
        Ok(())
    }
    fn load_recovery(&self, _key: &RecoveryKey) -> CadResult<Option<RecoverySnapshot>> {
        Ok(None)
    }
    fn recovery_hint(&self, _key: &RecoveryKey) -> Option<String> {
        Some("/tmp/recovery/0123.ymodel".to_string())
    }
    fn discard_recovery(&self, _key: &RecoveryKey) -> CadResult<()> {
        Ok(())
    }
}

/// The core, its archive and its files, with the policy already set to `embed`.
fn saving_core(
    label: &str,
    fonts: &[&str],
    policy: bool,
) -> (HostCore, Arc<CapturingArchive>, Rc<ResourceFiles>, Scratch) {
    let scratch = Scratch::new(label);
    let files = Rc::new(ResourceFiles::default());
    let core = resources_core(files.clone(), Some(scratch.path().to_path_buf()));
    install(&core, font_document(fonts));
    let archive = Arc::new(CapturingArchive::default());
    core.set_document_archive(Some(archive.clone()));
    if policy {
        assert!(
            core.set_embed_resources(true).expect("policy write"),
            "the flag must actually change"
        );
    }
    (core, archive, files, scratch)
}

// ---------------------------------------------------------------------------
// C1 (a): the flag off writes exactly what the document already was.
// ---------------------------------------------------------------------------

#[test]
fn a_document_without_the_flag_collects_nothing_and_does_not_move_the_revision() {
    let (core, archive, _files, _scratch) = saving_core("off", &[FONT_NAME], false);
    // The font exists and *would* resolve; only the policy stops the collection.
    write_font(_scratch.path(), FONT_NAME, FONT_BYTES);
    let before = core
        .controller
        .borrow()
        .snapshot_document()
        .expect("snapshot");
    let revision = before.revision;

    core.request_save().expect("save runs inside the gesture");

    let writes = archive.writes();
    assert_eq!(writes.len(), 1);
    // The durable save stamps persistence metadata (`document_times`) into the
    // header before encoding; that is the save path's clock, not the resource
    // collector. Everything else must be byte-for-byte the pre-save snapshot.
    let mut written = writes[0].clone();
    let expected = before.clone();
    assert!(
        written
            .header
            .variables
            .get("document_times")
            .is_some_and(|value| !value.is_null()),
        "a durable save stamps document_times"
    );
    assert_eq!(
        expected.header.variables.get("document_times"),
        None,
        "the pre-save snapshot stores no instants"
    );
    if let Some(map) = written.header.variables.as_object_mut() {
        map.remove("document_times");
    }
    assert_eq!(
        written, expected,
        "the snapshot the store encodes is byte-for-byte the pre-feature one"
    );
    assert!(writes[0].resources.is_empty(), "no payloads were added");
    assert!(
        writes[0].tables.get(TableKind::Resources).is_none(),
        "no mapping table either (D12)"
    );
    assert!(!writes[0].header.embed_resources);
    assert_eq!(
        core.controller
            .borrow()
            .drawing()
            .expect("drawing")
            .revision(),
        revision,
        "collection did not manufacture an edit"
    );
}

/// (a) A document that opted in but references nothing gets **no** extra text:
/// the ordinary save status is byte-identical to a document that never opted in.
#[test]
fn an_opted_in_document_with_nothing_to_embed_keeps_the_ordinary_save_status() {
    let scratch = Scratch::new("nothing");
    let files = Rc::new(ResourceFiles::default());
    let core = resources_core(files, Some(scratch.path().to_path_buf()));
    // A blank drawing: no font, no image, no external reference.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(9));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("layer");
    install(&core, builder.finish().expect("database"));
    let archive = Arc::new(CapturingArchive::default());
    core.set_document_archive(Some(archive.clone()));
    core.set_embed_resources(true).expect("policy write");

    core.request_save().expect("save");

    let writes = archive.writes();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].resources.is_empty(), "nothing was embedded");
    let status = core.last_status().expect("status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "save.exported_ymodel");
    assert_eq!(
        args.get("path").map(String::as_str),
        Some("plan.ymodel"),
        "the ordinary status line, unchanged"
    );
}

// ---------------------------------------------------------------------------
// C1 (b): one resolvable + one missing resource.
// ---------------------------------------------------------------------------

#[test]
fn a_resolvable_font_is_embedded_and_a_missing_one_is_reported_without_failing_the_save() {
    let (core, archive, files, scratch) = saving_core("one", &[FONT_NAME, MISSING_NAME], true);
    write_font(scratch.path(), FONT_NAME, FONT_BYTES);

    core.request_save()
        .expect("the save succeeds despite the miss");

    let writes = archive.writes();
    assert_eq!(writes.len(), 1, "the file was written");
    assert_eq!(files.written.borrow().len(), 1);
    let resources = &writes[0].resources;
    assert_eq!(resources.len(), 1, "exactly the one readable font");
    let ((kind, key), bytes) = resources.iter().next().expect("one entry");
    assert_eq!(*kind, cad_domain::BundledKind::Font);
    assert_eq!(bytes.as_ref(), FONT_BYTES);
    // D1: the key's stem is the first 32 hex of the payload's SHA-256.
    let expected = format!(
        "{}.ttf",
        &crate::sha256_hex(FONT_BYTES)[..cad_domain::resource::RESOURCE_KEY_HEX_LEN]
    );
    assert_eq!(key, &expected, "content-addressed key");
    assert!(cad_domain::is_valid_bundled_key(*kind, key));
    assert_eq!(
        writes[0]
            .resource_names
            .get(&(*kind, key.clone()))
            .map(String::as_str),
        Some(FONT_NAME),
        "the row carries the logical reference verbatim"
    );
    assert_eq!(
        writes[0]
            .resource_sources
            .get(&(*kind, key.clone()))
            .and_then(|source| source.as_deref()),
        Some(scratch.path().join(FONT_NAME).to_string_lossy().as_ref()),
        "the collection path is recorded for diagnostics"
    );
    assert_eq!(
        writes[0].tables.get(TableKind::Resources).map(<[_]>::len),
        Some(1),
        "one mapping row"
    );
    assert!(writes[0].header.embed_resources, "the document opted in");

    let Some(StatusMessage::ExternalText(text)) = core.last_status() else {
        panic!(
            "a save that embedded resources reports them: {:?}",
            core.last_status()
        );
    };
    assert!(text.contains("embedded=1"), "{text}");
    assert!(
        text.contains(&format!("bytes={}", FONT_BYTES.len())),
        "{text}"
    );
    assert!(
        text.contains(&format!("skip={MISSING_NAME}:resource.unavailable")),
        "the miss is named with its marker: {text}"
    );
    assert!(text.contains("path=plan.ymodel"), "{text}");
}

// ---------------------------------------------------------------------------
// C1 (c): the budget skips one entry and the save still succeeds.
// ---------------------------------------------------------------------------

#[test]
fn an_over_budget_entry_is_skipped_and_reported_while_the_rest_saves() {
    let (core, archive, files, scratch) = saving_core("budget", &[FONT_NAME, MISSING_NAME], true);
    write_font(scratch.path(), FONT_NAME, FONT_BYTES);
    // A sparse file just over the per-entry cap: `set_len` never writes 32 MiB.
    let huge = std::fs::File::create(scratch.path().join(MISSING_NAME)).expect("sparse file");
    huge.set_len(crate::save::RESOURCE_BUDGET_ENTRY_BYTES + 1)
        .expect("set_len");
    drop(huge);

    core.request_save()
        .expect("an over-budget entry must not fail the save");

    assert_eq!(files.written.borrow().len(), 1, "the file was written");
    let writes = archive.writes();
    assert_eq!(writes[0].resources.len(), 1, "the small font still travels");
    let Some(StatusMessage::ExternalText(text)) = core.last_status() else {
        panic!("expected the resource report, got {:?}", core.last_status());
    };
    assert!(
        text.contains(&format!("skip={MISSING_NAME}:resource.budget.entry")),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// C1 (e): content addressing makes the second save a no-op.
// ---------------------------------------------------------------------------

#[test]
fn a_second_save_does_not_advance_the_revision_again() {
    let (core, archive, files, scratch) = saving_core("idempotent", &[FONT_NAME], true);
    write_font(scratch.path(), FONT_NAME, FONT_BYTES);

    core.request_save().expect("first save");
    let after_first = {
        let controller = core.controller.borrow();
        let revision = controller.drawing().expect("drawing").revision();
        assert_eq!(
            controller
                .persistence()
                .expect("persistence")
                .saved_revision,
            revision,
            "the first save recorded the revision that embedded the font"
        );
        revision
    };

    core.request_save().expect("second save");

    let writes = archive.writes();
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[1].revision, after_first,
        "the second save did not bump the revision for an already-present key"
    );
    assert_eq!(
        core.controller
            .borrow()
            .drawing()
            .expect("drawing")
            .revision(),
        after_first
    );
    assert_eq!(writes[1].resources.len(), 1, "the resource is still there");
    assert_eq!(
        writes[0], writes[1],
        "an unchanged document re-encodes to the same snapshot"
    );
    assert!(
        !archive.scopes()[1].tables.contains(&TableKind::Resources),
        "nothing rewrote the mapping table: the store can copy it through"
    );
    // The document is the authority: once the font is embedded, deleting the host
    // file must not make the preview (or the next save) claim it is missing.
    std::fs::remove_file(scratch.path().join(FONT_NAME)).expect("remove the source font");
    let preview = core.ymodel_resource_preview().expect("archive installed");
    let row = preview
        .entries
        .iter()
        .find(|candidate| candidate.name == FONT_NAME)
        .expect("the referenced font is listed");
    assert!(row.found(), "the bytes live in the document, not the file");
    assert!(row.embedded);
    assert_eq!(row.size, FONT_BYTES.len() as u64);
    core.request_save().expect("third save");
    let writes = archive.writes();
    assert_eq!(
        writes[2].resources.len(),
        1,
        "the embedded font survives a missing host file"
    );
    assert_eq!(
        files.written.borrow().len(),
        3,
        "every save (two, plus the one after the source file was removed) wrote"
    );
}

// ---------------------------------------------------------------------------
// C3: the preview is read-only.
// ---------------------------------------------------------------------------

#[test]
fn the_preview_resolves_candidates_and_mutates_nothing() {
    let (core, archive, _files, scratch) = saving_core("preview", &[FONT_NAME, MISSING_NAME], true);
    write_font(scratch.path(), FONT_NAME, FONT_BYTES);
    // A resource this build cannot name stays a miss instead of a guess (D1).
    write_font(scratch.path(), "notes.txt", b"not a font");
    // Replacing the document drops the flag with the old document, so it is set
    // again here (D3: the flag is a document property, not a host preference).
    install(
        &core,
        font_document(&[FONT_NAME, MISSING_NAME, "notes.txt"]),
    );
    core.set_embed_resources(true).expect("policy write");

    // State *before* the read: the policy write above is a real edit, so the
    // "no side effect" claim is that the preview changes none of it.
    let (revision, scope, writes_before) = {
        let controller = core.controller.borrow();
        (
            controller.drawing().expect("drawing").revision(),
            controller.dirty_scope().clone(),
            archive.writes().len(),
        )
    };

    let preview = core
        .ymodel_resource_preview()
        .expect("the archive is installed");
    assert!(preview.policy, "the document opted in");
    assert_eq!(preview.entries.len(), 3, "one row per referenced name");
    let by_name = |name: &str| {
        preview
            .entries
            .iter()
            .find(|candidate| candidate.name == name)
            .unwrap_or_else(|| panic!("{name} is listed"))
    };
    let found = by_name(FONT_NAME);
    assert!(found.found(), "the readable font is a hit");
    assert_eq!(found.size, FONT_BYTES.len() as u64);
    assert!(found.source.is_some(), "the row says where to read it");
    let missing = by_name(MISSING_NAME);
    assert!(!missing.found());
    assert_eq!(missing.size, 0);
    assert_eq!(missing.reason, Some("resource.unavailable"));
    let unsupported = by_name("notes.txt");
    assert!(
        !unsupported.found(),
        "an unnameable extension is not embeddable"
    );
    assert_eq!(unsupported.reason, Some("resource.unsupported_kind"));

    // Side-effect free: no revision, no scope change, no archive write — even
    // when the preview is read twice.
    let _again = core.ymodel_resource_preview().expect("second read");
    let controller = core.controller.borrow();
    assert_eq!(controller.drawing().expect("drawing").revision(), revision);
    assert_eq!(controller.dirty_scope(), &scope);
    drop(controller);
    assert_eq!(
        archive.writes().len(),
        writes_before,
        "the preview never encodes"
    );
}

#[test]
fn the_preview_refuses_on_a_carrier_without_an_archive() {
    let scratch = Scratch::new("no-archive");
    let core = resources_core(
        Rc::new(ResourceFiles::default()),
        Some(scratch.path().to_path_buf()),
    );
    install(&core, font_document(&[FONT_NAME]));
    let error = core
        .ymodel_resource_preview()
        .expect_err("no archive means no ymodel file to describe");
    assert_eq!(error_text(&error), "unsupported-runtime: ymodel");
    assert_eq!(
        error_text(&core.set_embed_resources(true).expect_err("no archive")),
        "unsupported-runtime: ymodel",
        "the setter refuses too instead of storing an unpersistable flag"
    );
}

// ---------------------------------------------------------------------------
// C4: the recovery snapshot never packs resources.
// ---------------------------------------------------------------------------

#[test]
fn a_recovery_snapshot_drops_the_documents_resources_and_says_so() {
    let (mut core, archive, _files, scratch) = saving_core("recovery", &[FONT_NAME], true);
    write_font(scratch.path(), FONT_NAME, FONT_BYTES);
    core.request_save().expect("save embeds the font");
    assert_eq!(
        archive
            .writes()
            .last()
            .map(|snapshot| snapshot.resources.len()),
        Some(1),
        "the saved document really carries the resource"
    );

    let store = Arc::new(QuietPersistence::default());
    core.set_persistence(Some(store.clone()));
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 5.0), p(5.0, 5.0))
        .expect("edit makes the document dirty");
    assert!(core.controller.borrow().is_dirty());

    // A deterministic clock: the first tick arms the interval, the second is due.
    let now = Rc::new(Cell::new(0_u64));
    let clock = now.clone();
    let base = std::time::Instant::now();
    core.set_recovery_clock_for_test(Rc::new(move || {
        base + std::time::Duration::from_millis(clock.get())
    }));
    core.tick_recovery();
    now.set(RECOVERY_INTERVAL.as_millis() as u64);
    core.tick_recovery();

    assert_eq!(store.saved.lock().unwrap().len(), 1, "one snapshot written");
    let encoded = archive.writes();
    let recovery = encoded.last().expect("the recovery encode");
    assert!(
        recovery.resources.is_empty(),
        "a recovery snapshot carries no resource payloads (D11)"
    );
    assert!(recovery.resource_names.is_empty() && recovery.resource_sources.is_empty());
    assert!(
        recovery.tables.get(TableKind::Resources).is_none(),
        "and no mapping table, which would name entries the archive lacks"
    );
    assert!(!recovery.header.embed_resources, "the flag is cleared too");

    let Some(StatusMessage::ExternalText(text)) = core.last_status() else {
        panic!("the omission is surfaced, got {:?}", core.last_status());
    };
    assert!(
        text.starts_with(cad_domain::ymodel::markers::RECOVERY_RESOURCES_OMITTED),
        "{text}"
    );

    // Latched: a second snapshot in the same document does not repeat the line.
    core.set_external_status("reset").expect("status");
    now.set(2 * RECOVERY_INTERVAL.as_millis() as u64);
    core.tick_recovery();
    assert_eq!(store.saved.lock().unwrap().len(), 2, "a second snapshot");
    assert!(matches!(
        core.last_status(),
        Some(StatusMessage::ExternalText(text)) if text == "reset"
    ));
}
