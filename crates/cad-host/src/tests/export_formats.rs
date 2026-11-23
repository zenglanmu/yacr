//! `EXPORT`'s suggested name and the typed `EXPORT <TOKEN>` writer contract
//! (ADR-0052).
//!
//! The bare command keeps the all-format picker but suggests the **source
//! format's own** extension when that format is a non-experimental write-back
//! format; a typed token fixes the writer for one interaction while the chosen
//! target's extension stays the final authority (`export.format_mismatch`).
//! Split into its own module to keep `file_open_save.rs` inside the code-size
//! gate.

use super::*;

use std::path::Path;

/// A save platform whose chosen target (when set) is synthesized synchronously,
/// recording the picker's suggested name/purpose and every write.
#[derive(Default)]
struct ExportFiles {
    calls: RefCell<Vec<(SavePurpose, String)>>,
    events: RefCell<Vec<FileEvent>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
    chosen: RefCell<Option<FileTarget>>,
}

impl ExportFiles {
    fn new() -> Self {
        Self::default()
    }
}

impl PlatformFiles for ExportFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        self.calls
            .borrow_mut()
            .push((purpose, default_name.to_string()));
        if let Some(target) = self.chosen.borrow().clone() {
            self.events
                .borrow_mut()
                .push(FileEvent::SaveTarget { purpose, target });
        }
        Ok(())
    }
    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        self.written
            .borrow_mut()
            .push((target.display_name.clone(), bytes.to_vec()));
        Ok(())
    }
    fn set_source(&self, _target: Option<&FileTarget>) {}
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

#[test]
fn export_suggests_a_writable_source_format_extension() {
    // The hint is the label the host records from the opened source and the only
    // input of the suggested name; the writer still comes from the chosen target.
    for (source, expected) in [
        ("/tmp/mesh.glb", "mesh.glb"),
        ("/tmp/model.ifc", "model.ifc"),
        ("/tmp/part.obj", "part.obj"),
        ("/tmp/part.COFF", "part.coff"),
        ("/tmp/tokens.gltf", "tokens.gltf"),
    ] {
        let files = Rc::new(ExportFiles::new());
        let core = core_with_caps(files.clone(), capabilities(false));
        core.controller.borrow_mut().document_name_hint = source.to_string();
        core.request_export().unwrap();
        let calls = files.calls.borrow();
        assert_eq!(calls.len(), 1, "{source}: exactly one picker");
        assert_eq!(calls[0].0, SavePurpose::SaveExport, "{source}");
        assert_eq!(calls[0].1, expected, "{source}: suggested name");
    }
}

#[test]
fn an_experimental_read_only_or_blank_source_still_suggests_dxf() {
    // Experimental writers (ADR-0041 D5: DWG/SKP), the native archive, read-only
    // formats and a blank/extensionless label all fall back to `.dxf`.
    for (source, expected) in [
        ("/tmp/plan.dwg", "plan.dxf"),
        ("/tmp/plan.SKP", "plan.dxf"),
        ("/tmp/plan.ymodel", "plan.dxf"),
        ("/tmp/plan.step", "plan.dxf"),
        ("/tmp/plan.dgn", "plan.dxf"),
        ("/tmp/plan.rvt", "plan.dxf"),
        ("/tmp/plan", "plan.dxf"),
        // An empty label has no stem: the documented `drawing` fallback.
        ("", "drawing.dxf"),
    ] {
        let files = Rc::new(ExportFiles::new());
        let core = core_with_caps(files.clone(), capabilities(false));
        core.controller.borrow_mut().document_name_hint = source.to_string();
        core.request_export().unwrap();
        let calls = files.calls.borrow();
        assert_eq!(calls.len(), 1, "{source}: exactly one picker");
        assert_eq!(calls[0].1, expected, "{source}: the dxf fallback");
    }
}

/// Open the committed STL mesh fixture as a mesh document with resolved metres.
fn open_mesh_fixture(core: &HostCore) {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/stl/synthetic-quad.stl");
    let bytes = std::fs::read(&fixture).expect("committed STL fixture");
    core.deliver_external_open(FileTarget::from_path("/tmp/source.stl"), Arc::from(bytes))
        .expect("the STL fixture imports as a mesh document");
    // glTF positions are metres, so the writer refuses an unknown unit.
    core.controller
        .borrow_mut()
        .set_units(cad_domain::UnitContext::from_meter())
        .unwrap();
}

#[test]
fn a_forced_glb_export_writes_the_gltf_connector_for_a_glb_target() {
    let files = Rc::new(ExportFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/forced.glb"));
    let core = core_with_caps(files.clone(), capabilities(false));
    open_mesh_fixture(&core);

    core.request_export_format("glb", "glb")
        .expect("the typed export opens the picker");
    assert!(
        !core.save_pending.get(),
        "the synchronous target is drained"
    );
    {
        let calls = files.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1, "source.glb",
            "the picker suggests the token's own extension"
        );
    }
    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one file is written");
    assert_eq!(written[0].0, "/tmp/forced.glb");
    assert!(
        written[0].1.starts_with(b"glTF"),
        "the forced writer still produces the binary glTF container"
    );
    drop(written);
    let status = core.last_status().expect("the export grades a status");
    let (key, args) = catalog(&status);
    assert!(
        key == "save.exported_connector" || key == "save.exported_connector_partial",
        "the status names the connector export: {key}"
    );
    assert_eq!(args.get("format"), Some(&"glb".to_string()));
    assert!(
        core.pending_export_format.borrow().is_none(),
        "the forced writer is consumed by its one interaction"
    );
}

#[test]
fn a_forced_format_refuses_a_mismatched_target_before_writing() {
    let files = Rc::new(ExportFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/forced.dxf"));
    let core = core_with_caps(files.clone(), capabilities(false));
    open_mesh_fixture(&core);

    core.request_export_format("glb", "glb")
        .expect("the picker interaction completes");
    assert!(
        files.written.borrow().is_empty(),
        "a mismatched target writes nothing"
    );
    let status = core.last_status().expect("the refusal is graded");
    let (key, args) = catalog(&status);
    assert_eq!(key, "save.export_failed");
    let reason = args.get("reason").expect("the refusal names its reason");
    assert!(
        reason.starts_with("export.format_mismatch: requested glb, target .dxf is dxf"),
        "the stable marker names both sides: {reason}"
    );
    assert!(
        core.pending_export_format.borrow().is_none(),
        "a refused interaction leaves no forced writer behind"
    );
}

#[test]
fn a_bare_export_after_a_forced_one_forces_nothing() {
    // The forced writer never leaks: a later bare EXPORT is back to the chosen
    // target's extension as the only authority.
    let files = Rc::new(ExportFiles::new());
    let core = core_with_caps(files.clone(), capabilities(false));
    open_mesh_fixture(&core);
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/forced.glb"));
    core.request_export_format("glb", "glb").unwrap();
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/plain.stl"));
    core.request_export().unwrap();

    let written = files.written.borrow();
    assert_eq!(written.len(), 2, "both exports wrote");
    assert_eq!(written[1].0, "/tmp/plain.stl");
    assert!(
        !written[1].1.starts_with(b"glTF"),
        "the bare export wrote STL, not the previously forced glTF"
    );
}
