//! Recent-drawings host contracts: recording, the durable root array, the
//! clear, and the reopen funnel.
//!
//! These are synthetic platform contracts (no real dialog, no GPU): the fake
//! platform delivers bytes exactly as the real open funnel does.

use super::*;

use crate::recent::{record_success, RecentEntry, RECENT_FILES_CAP};
use cad_platform::FilePurpose;
use std::cell::Cell;

/// A config store that loads one seeded preference document and records every
/// persisted projection; `fail` makes the next writes fail explicitly.
struct SeededConfig {
    preference: String,
    persisted: RefCell<Vec<String>>,
    fail: Cell<bool>,
}

impl SeededConfig {
    fn new(preference: impl Into<String>) -> Rc<Self> {
        Rc::new(Self {
            preference: preference.into(),
            persisted: RefCell::new(Vec::new()),
            fail: Cell::new(false),
        })
    }

    fn last_persisted(&self) -> serde_json::Value {
        let text = self
            .persisted
            .borrow()
            .last()
            .cloned()
            .expect("a preference write happened");
        serde_json::from_str(&text).expect("the persisted preference is JSON")
    }
}

impl ConfigStore for SeededConfig {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }

    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Text {
            path: "/test/preference.json".to_string(),
            text: self.preference.clone(),
        }
    }

    fn persist_preference(&self, text: &str) -> CadResult<()> {
        if self.fail.get() {
            return Err(CadError::Unsupported("the disk is full".into()));
        }
        self.persisted.borrow_mut().push(text.to_string());
        Ok(())
    }
}

fn core_with(files: Rc<FakeFiles>, config: Rc<dyn ConfigStore>) -> HostCore {
    core_with_resources(files, config, Rc::new(UnsupportedResources))
}

fn core_with_resources(
    files: Rc<FakeFiles>,
    config: Rc<dyn ConfigStore>,
    resources: Rc<dyn HostResources>,
) -> HostCore {
    let platform = HostPlatform {
        files,
        config_store: config,
        surface: Rc::new(FakeSurface),
        resources,
        capabilities: capabilities(false),
    };
    HostCore::new(options(), platform, RenderSessionHandle::default()).unwrap()
}

/// A resource strategy that installs nothing and succeeds, so a later status is
/// not overwritten by the fixture's explicit font/image refusal.
struct NoopResources;

impl HostResources for NoopResources {
    fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }

    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
}

fn target(name: &str) -> FileTarget {
    FileTarget {
        display_name: name.to_string(),
        opaque: format!("/tmp/drawings/{name}"),
    }
}

/// Deliver one successfully importable document through the real open funnel.
fn deliver_success(core: &HostCore, files: &Rc<FakeFiles>, target: FileTarget) {
    let bytes = core.controller.borrow().export_dxf().unwrap().0;
    files.events.borrow_mut().push(FileEvent::Opened {
        target,
        bytes: Arc::from(bytes),
    });
    core.drain_files().expect("the valid document installs");
}

#[test]
fn a_successful_open_records_exactly_once_with_the_platform_hint() {
    let files = Rc::new(FakeFiles::default());
    *files.location_hint.borrow_mut() = Some("/tmp/drawings".to_string());
    let config = SeededConfig::new("{}");
    let core = core_with(files.clone(), config.clone());
    core.load_startup();

    let opened = target("plan.dwg");
    deliver_success(&core, &files, opened.clone());

    let entries = core.recent_entries();
    assert_eq!(entries.len(), 1, "one successful open records one row");
    assert_eq!(entries[0].display_name, "plan.dwg");
    assert_eq!(entries[0].opaque, opened.opaque);
    assert_eq!(entries[0].location.as_deref(), Some("/tmp/drawings"));
    assert!(entries[0].opened_at_ms > 0, "a real wall-clock stamp");
    assert_eq!(
        files.recent_target.borrow().as_ref(),
        Some(&opened),
        "the next chooser starts from the successful target"
    );

    // The durable write is the same list, with the root key and camelCase rows.
    let persisted = config.last_persisted();
    let rows = persisted
        .get("recentFiles")
        .and_then(serde_json::Value::as_array)
        .expect("the recentFiles array is persisted");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("displayName").and_then(|value| value.as_str()),
        Some("plan.dwg")
    );
    assert_eq!(
        rows[0].get("openedAtMs").and_then(|value| value.as_u64()),
        Some(entries[0].opened_at_ms)
    );

    // A later tick with no new event cannot record the same open twice.
    core.drain_files().unwrap();
    core.poll_loading().unwrap();
    assert_eq!(core.recent_entries().len(), 1);
}

#[test]
fn failed_and_cancelled_deliveries_record_nothing() {
    let files = Rc::new(FakeFiles::default());
    let config = SeededConfig::new("{}");
    let core = core_with(files.clone(), config);
    core.load_startup();

    files.events.borrow_mut().push(FileEvent::Failed {
        purpose: FilePurpose::Open,
        reason: "portal down".into(),
    });
    core.drain_files().unwrap();
    files.events.borrow_mut().push(FileEvent::Cancelled {
        purpose: FilePurpose::Open,
    });
    core.drain_files().unwrap();

    // A delivered but unparseable document is a failed open too.
    files.events.borrow_mut().push(FileEvent::Opened {
        target: target("broken.dwg"),
        bytes: Arc::from(vec![0u8, 1, 2, 3]),
    });
    let _ = core.drain_files();

    assert!(
        core.recent_entries().is_empty(),
        "no failure is a recent row"
    );
    assert!(files.recent_target.borrow().is_none());
}

#[test]
fn the_list_is_deduped_newest_first_and_capped() {
    let mut entries: Vec<RecentEntry> = Vec::new();
    for index in 0..(RECENT_FILES_CAP + 2) {
        record_success(
            &mut entries,
            &target(&format!("draw-{index}.dwg")),
            None,
            index as u64,
        );
    }
    assert_eq!(entries.len(), RECENT_FILES_CAP, "the cap is enforced");
    assert_eq!(entries[0].display_name, "draw-11.dwg", "newest first");
    assert_eq!(entries[0].opened_at_ms, 11);

    // Re-opening an existing handle moves it to the front with the new stamp and
    // leaves exactly one row for that handle.
    record_success(&mut entries, &target("draw-5.dwg"), Some("/tmp".into()), 99);
    assert_eq!(entries.len(), RECENT_FILES_CAP);
    assert_eq!(entries[0].opaque, "/tmp/drawings/draw-5.dwg");
    assert_eq!(entries[0].opened_at_ms, 99);
    assert_eq!(entries[0].location.as_deref(), Some("/tmp"));
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.opaque == "/tmp/drawings/draw-5.dwg")
            .count(),
        1
    );
}

#[test]
fn an_unrelated_preference_write_keeps_the_recent_files_leaf() {
    let files = Rc::new(FakeFiles::default());
    let config = SeededConfig::new("{}");
    let core = core_with(files.clone(), config.clone());
    core.load_startup();
    deliver_success(&core, &files, target("plan.dwg"));

    // A theme write re-projects the whole document; the recent array must ride
    // along unchanged.
    core.root_preferences.borrow_mut().theme = Some("light".to_string());
    core.persist_preference().unwrap();

    let persisted = config.last_persisted();
    assert_eq!(
        persisted.get("theme").and_then(|v| v.as_str()),
        Some("light")
    );
    assert_eq!(
        persisted
            .get("recentFiles")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "an unrelated write cannot drop the user's recent rows"
    );
}

#[test]
fn a_malformed_array_is_an_empty_list_never_a_partial_salvage() {
    let malformed = r#"{"recentFiles":[{"displayName":"ok.dwg","opaque":"/tmp/ok.dwg","location":null,"openedAtMs":1},{"displayName":"broken.dwg"}]}"#;
    let files = Rc::new(FakeFiles::default());
    let config = SeededConfig::new(malformed);
    let core = core_with(files.clone(), config);
    let diagnostics = core.load_startup();

    assert!(
        core.recent_entries().is_empty(),
        "one bad row makes the list untrusted"
    );
    assert!(
        files.recent_target.borrow().is_none(),
        "no hint from an untrusted list"
    );
    assert!(
        diagnostics.iter().all(|line| !line.contains("panic")),
        "the load stays a real diagnostic path: {diagnostics:?}"
    );

    // A non-array value is equally untrusted (not a one-row list).
    for preference in [
        r#"{"recentFiles":{}}"#,
        r#"{"recentFiles":"/tmp/plan.dwg"}"#,
        "not json at all",
    ] {
        let files = Rc::new(FakeFiles::default());
        let core = core_with(files, SeededConfig::new(preference));
        core.load_startup();
        assert!(core.recent_entries().is_empty(), "{preference}");
    }
}

#[test]
fn startup_seeds_the_list_and_the_next_chooser_hint() {
    let preference = r#"{"recentFiles":[
        {"displayName":"newest.dwg","opaque":"/tmp/newest.dwg","location":"/tmp","openedAtMs":20},
        {"displayName":"older.dwg","opaque":"/tmp/older.dwg","location":"/tmp","openedAtMs":10}
    ]}"#;
    let files = Rc::new(FakeFiles::default());
    let config = SeededConfig::new(preference);
    let core = core_with(files.clone(), config.clone());
    core.load_startup();

    let entries = core.recent_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].display_name, "newest.dwg");
    assert_eq!(
        files
            .recent_target
            .borrow()
            .as_ref()
            .map(|target| target.opaque.as_str()),
        Some("/tmp/newest.dwg"),
        "the chooser starts from the newest durable row"
    );
    // The startup re-persist keeps the array (it is not dropped as unknown).
    assert_eq!(
        config
            .last_persisted()
            .get("recentFiles")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn clear_drops_the_leaf_and_rolls_back_on_a_failed_write() {
    let files = Rc::new(FakeFiles::default());
    let config = SeededConfig::new("{}");
    let core = core_with(files.clone(), config.clone());
    core.load_startup();
    deliver_success(&core, &files, target("plan.dwg"));
    assert_eq!(core.recent_entries().len(), 1);

    core.clear_user_preference().unwrap();
    assert!(core.recent_entries().is_empty());
    assert!(
        config.last_persisted().get("recentFiles").is_none(),
        "a real clear removes the leaf"
    );

    // A failed write restores the rows instead of pretending they are gone.
    deliver_success(&core, &files, target("again.dwg"));
    assert_eq!(core.recent_entries().len(), 1);
    config.fail.set(true);
    assert!(core.clear_user_preference().is_err());
    assert_eq!(
        core.recent_entries().len(),
        1,
        "a clear that could not persist restores the list"
    );
}

#[test]
fn reopen_uses_the_stored_handle_and_refuses_an_unknown_one() {
    let preference = r#"{"recentFiles":[
        {"displayName":"plan.dwg","opaque":"saf://handle/42","location":null,"openedAtMs":20}
    ]}"#;
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), SeededConfig::new(preference));
    core.load_startup();

    let error = core
        .open_recent("saf://handle/other", "crafted.dwg")
        .expect_err("a handle outside the list is refused");
    assert!(
        error.to_string().contains("recent.unknown"),
        "the refusal is explicit: {error:?}"
    );
    assert!(files.opened_target.borrow().is_none(), "nothing was opened");

    core.open_recent("saf://handle/42", "plan.dwg").unwrap();
    assert_eq!(
        files.opened_target.borrow().as_ref(),
        Some(&FileTarget {
            display_name: "plan.dwg".to_string(),
            opaque: "saf://handle/42".to_string(),
        })
    );
    assert!(core.open_pending.get(), "the platform read is in flight");
}

#[test]
fn a_persistence_failure_is_a_status_error_and_rolls_the_row_back() {
    let files = Rc::new(FakeFiles::default());
    let config = SeededConfig::new("{}");
    let core = core_with_resources(files.clone(), config.clone(), Rc::new(NoopResources));
    core.load_startup();
    config.fail.set(true);

    // The document still opens; only the durable recent row fails.
    deliver_success(&core, &files, target("plan.dwg"));
    assert!(
        core.recent_entries().is_empty(),
        "a row the next start would lose is not kept in memory"
    );
    match core.last_status() {
        Some(StatusMessage::ExternalText(text)) => assert!(
            text.contains("post-commit state sync failed"),
            "the failure is a real status error: {text}"
        ),
        other => panic!("expected an explicit status error, got {other:?}"),
    }
}
