//! The `cad_fonts` IPC surface: the explicit "no session" answer, the exact
//! camelCase report shape the read-only Font Manager codes against, and the
//! `available: false` state (a named reason, never an empty registry).
//!
//! The report itself is projected by the owner-thread session
//! (`cad_ui_core` tests: `a_real_report_is_projected_one_row_per_host_face`,
//! `no_report_is_an_available_empty_answer_with_the_configured_directory`); this
//! file pins the transport: the mirror is the only source, `cad_waiting` is not
//! an empty answer, and the generated command is registered.

use cad_desktop_shell::commands::{cad_fonts_impl, COMMAND_NAMES};
use cad_desktop_shell::state::{lock, new_shared};
use cad_ui_core::launch::LaunchOptions;
use cad_ui_core::session::{CadFont, CadFonts};
use serde_json::json;

fn cad_launch() -> LaunchOptions {
    LaunchOptions::default()
}

fn face(key: &str, status: &str, source: &str) -> CadFont {
    CadFont {
        key: key.to_string(),
        file: Some(format!("{key}.shx")),
        kind: "shx".to_string(),
        source: source.to_string(),
        status: status.to_string(),
        detail: None,
    }
}

#[test]
fn cad_fonts_is_registered_next_to_the_other_control_reads() {
    for name in ["cad_settings", "cad_layers", "cad_fonts"] {
        assert!(
            COMMAND_NAMES.contains(&name),
            "{name} is not registered in cad-desktop-shell"
        );
    }
}

#[test]
fn cad_fonts_refuses_explicitly_before_a_session_exists() {
    let shared = new_shared(cad_launch());
    let error = cad_fonts_impl(&shared).expect_err("no session yet");
    assert_eq!(error, "cad_waiting");
}

#[test]
fn cad_fonts_serializes_the_mirrored_report_in_camel_case() {
    let shared = new_shared(cad_launch());
    {
        let mut state = lock(&shared);
        state.cad_fonts = Some(Ok(CadFonts {
            available: true,
            reason: None,
            fonts_dir: Some("/opt/yacr/fonts".into()),
            catalog_entries: 4,
            locale: "zh-Hans".into(),
            default_face: Some("romans".into()),
            locale_missing: Some("zh-Hans (simsun, gbcbig)".into()),
            requested: vec!["romans".into()],
            text_needed: true,
            faces: vec![
                face("romans", "loaded", "package"),
                CadFont {
                    key: "HZTXT".into(),
                    file: None,
                    kind: "system".into(),
                    source: "system".into(),
                    status: "unresolved".into(),
                    detail: Some("missing glyph".into()),
                },
            ],
        }));
    }

    let fonts = cad_fonts_impl(&shared).expect("the mirrored snapshot");
    let value = serde_json::to_value(&fonts).expect("serializes");
    assert_eq!(
        value,
        json!({
            "available": true,
            "reason": null,
            "fontsDir": "/opt/yacr/fonts",
            "catalogEntries": 4,
            "locale": "zh-Hans",
            "defaultFace": "romans",
            "localeMissing": "zh-Hans (simsun, gbcbig)",
            "requested": ["romans"],
            "textNeeded": true,
            "faces": [
                {
                    "key": "romans",
                    "file": "romans.shx",
                    "kind": "shx",
                    "source": "package",
                    "status": "loaded",
                    "detail": null
                },
                {
                    "key": "HZTXT",
                    "file": null,
                    "kind": "system",
                    "source": "system",
                    "status": "unresolved",
                    "detail": "missing glyph"
                }
            ]
        })
    );
}

#[test]
fn an_empty_report_keeps_every_key() {
    let value = serde_json::to_value(CadFonts::empty()).expect("serializes");
    assert_eq!(
        value,
        json!({
            "available": true,
            "reason": null,
            "fontsDir": null,
            "catalogEntries": 0,
            "locale": "latin",
            "defaultFace": null,
            "localeMissing": null,
            "requested": [],
            "textNeeded": false,
            "faces": []
        })
    );
}

#[test]
fn an_unavailable_answer_names_its_reason_and_invents_no_rows() {
    let shared = new_shared(cad_launch());
    {
        let mut state = lock(&shared);
        state.cad_fonts = Some(Ok(CadFonts::unsupported("host.fonts_unavailable")));
    }
    let value = serde_json::to_value(cad_fonts_impl(&shared).expect("the mirror")).expect("json");
    assert_eq!(value["available"], json!(false));
    assert_eq!(value["reason"], json!("host.fonts_unavailable"));
    assert_eq!(value["faces"], json!([]));
}

#[test]
fn a_projection_error_is_relayed_not_replaced_by_an_empty_registry() {
    let shared = new_shared(cad_launch());
    {
        let mut state = lock(&shared);
        state.cad_fonts = Some(Err("font.projection_failed: boom".into()));
    }
    let error = cad_fonts_impl(&shared).expect_err("an error is not an empty success");
    assert!(error.contains("font.projection_failed"), "{error}");
}
