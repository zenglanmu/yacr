//! The read-only font-report projection (`cad_fonts`).
//!
//! The host owns the face registry; these contracts only pin the wire mapping:
//! an absent report is an explicit available-empty answer (with the configured
//! directory), and a real report is projected one row per real face with stable
//! machine labels and no invented row.

use super::super::*;
use super::test_support::*;

use cad_platform::fonts::{
    FontFaceRecord, FontFaceSource, FontFaceStatus, FontLoadReport, LocaleFallbackGap, TextLocale,
};

use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer, Style};
use cad_domain::{
    DatabaseId, LayerId, ObjectId, Revision, SpaceId, StyleId, TextAlignH, TextAlignV,
};
use cad_engine::protocol::EngineClient;
use cad_host::HostResources;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn face(
    key: &str,
    file: Option<&str>,
    kind: &str,
    source: FontFaceSource,
    status: FontFaceStatus,
    detail: Option<&str>,
) -> FontFaceRecord {
    FontFaceRecord {
        key: key.to_string(),
        file: file.map(str::to_string),
        kind: kind.to_string(),
        source,
        status,
        detail: detail.map(str::to_string),
    }
}

#[test]
fn no_report_is_an_available_empty_answer_with_the_configured_directory() {
    let mut session = fresh_session();
    session.core.options.fonts_dir = Some(std::path::PathBuf::from("/opt/yacr/fonts"));
    // The fixture installs through `FakeResources`, which publishes no report.
    assert!(session.core.last_font_report().is_none());

    let answer = session.fonts().expect("the empty state is not an error");
    assert!(answer.available);
    assert_eq!(answer.reason, None);
    assert_eq!(answer.fonts_dir.as_deref(), Some("/opt/yacr/fonts"));
    assert_eq!(answer.catalog_entries, 0);
    assert_eq!(answer.locale, "latin");
    assert_eq!(answer.default_face, None);
    assert_eq!(answer.locale_missing, None);
    assert!(
        answer.requested.is_empty(),
        "the fixture drawing names no font"
    );
    assert!(!answer.text_needed, "the fixture drawing holds no text");
    assert!(answer.faces.is_empty(), "nothing is invented");
}

#[test]
fn a_real_report_is_projected_one_row_per_host_face() {
    let session = fresh_session();
    let report = FontLoadReport {
        catalog_entries: 4,
        requested: vec!["romans".into()],
        locale: TextLocale::SimplifiedChinese,
        default_face: Some("romans".into()),
        locale_missing: Some(LocaleFallbackGap {
            locale: TextLocale::SimplifiedChinese,
            candidates: vec!["simsun".into(), "gbcbig".into()],
        }),
        faces: vec![
            face(
                "romans",
                Some("romans.shx"),
                "shx",
                FontFaceSource::Package,
                FontFaceStatus::Loaded,
                None,
            ),
            face(
                "HZTXT",
                Some("hztxt.shx"),
                "shx",
                FontFaceSource::Explicit,
                FontFaceStatus::Failed,
                Some("missing glyph"),
            ),
            face(
                "Arial",
                None,
                "system",
                FontFaceSource::System,
                FontFaceStatus::Unresolved,
                None,
            ),
            face(
                "old-mesh",
                None,
                "mesh",
                FontFaceSource::Package,
                FontFaceStatus::Unsupported,
                Some("mesh faces are not rendered"),
            ),
        ],
        ..FontLoadReport::default()
    };
    session.core.set_last_font_report(report);

    let answer = session.fonts().expect("the report projects");
    assert!(answer.available);
    assert_eq!(answer.reason, None);
    assert_eq!(answer.catalog_entries, 4);
    assert_eq!(answer.locale, "zh-Hans");
    assert_eq!(answer.default_face.as_deref(), Some("romans"));
    assert_eq!(
        answer.locale_missing.as_deref(),
        Some("zh-Hans (simsun, gbcbig)"),
        "the gap detail names the locale and every candidate, as machine text"
    );
    assert_eq!(
        answer.requested,
        vec!["romans".to_string()],
        "a report's own requested keys are used when it exists"
    );
    assert!(
        !answer.text_needed,
        "the report carries no text flag; it comes from the drawing"
    );
    // Exactly the host's rows, in the host's order (nothing coalesced or sorted).
    assert_eq!(answer.faces.len(), 4);
    let labels: Vec<(&str, &str, &str, Option<&str>)> = answer
        .faces
        .iter()
        .map(|row| {
            (
                row.key.as_str(),
                row.source.as_str(),
                row.status.as_str(),
                row.detail.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        labels,
        vec![
            ("romans", "package", "loaded", None),
            ("HZTXT", "explicit", "failed", Some("missing glyph")),
            ("Arial", "system", "unresolved", None),
            (
                "old-mesh",
                "package",
                "unsupported",
                Some("mesh faces are not rendered")
            ),
        ]
    );
    assert_eq!(answer.faces[0].file.as_deref(), Some("romans.shx"));
    assert_eq!(answer.faces[2].file, None);
    assert_eq!(answer.faces[0].kind, "shx");
    assert_eq!(answer.faces[3].kind, "mesh");
}

#[test]
fn the_wire_answer_keeps_every_camel_case_key() {
    let value = serde_json::to_value(CadFonts::empty()).expect("serializes");
    assert_eq!(
        value,
        serde_json::json!({
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
    let unavailable =
        serde_json::to_value(CadFonts::unsupported("host.fonts_unavailable")).expect("serializes");
    assert_eq!(unavailable["available"], serde_json::json!(false));
    assert_eq!(
        unavailable["reason"],
        serde_json::json!("host.fonts_unavailable")
    );
    assert_eq!(unavailable["requested"], serde_json::json!([]));
    assert_eq!(unavailable["textNeeded"], serde_json::json!(false));
}

/// A drawing naming one font through its style, plus one text entity whose own
/// font is also named: both keys are what a lazy carrier must install.
fn text_bearing_drawing() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("the layer inserts");
    builder
        .insert_style(Style {
            id: StyleId(7),
            name: "Standard".into(),
            resource_keys: vec!["bigfont.shx".into()],
        })
        .expect("the style inserts");
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbText".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: cad_domain::EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: cad_domain::SemanticGeometry::Text {
                text: "hello".into(),
                position: cad_domain::Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                style: StyleId(7),
                height: 2.5,
                rotation: 0.0,
                font: Some("simplex.shx".into()),
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
                wrap_width: None,
            },
            draw_order: 0,
        })
        .expect("the text entity inserts");
    builder.finish().expect("the drawing builds")
}

/// Install an authoritative drawing directly (tests only); the document slot is
/// unchanged, exactly like the shared layout/xref fixtures.
fn install_drawing(session: &CadSession, drawing: DrawingDatabase) {
    let mut controller = session.core.controller.borrow_mut();
    let document_id = controller.document_id;
    controller
        .application
        .workspace
        .documents
        .get_mut(&document_id)
        .expect("the session's document")
        .drawing = Arc::new(drawing);
}

#[test]
fn no_package_still_reports_what_the_drawing_needs() {
    // The fixture publishes no report (no package installed yet), yet the
    // drawing's own needs must already be visible to the lazy loader.
    let session = fresh_session();
    install_drawing(&session, text_bearing_drawing());
    assert!(session.core.last_font_report().is_none());

    let answer = session.fonts().expect("the empty state is not an error");
    assert!(answer.available);
    assert_eq!(answer.fonts_dir, None);
    assert!(
        answer.faces.is_empty(),
        "no package means no face rows, never an invented one"
    );
    assert_eq!(
        answer.requested,
        vec!["simplex.shx".to_string(), "bigfont.shx".to_string()],
        "the entity font followed by its style's resource keys"
    );
    assert!(answer.text_needed, "the text needs shaping");
}

/// A strategy that refuses on every install after the first (construction)
/// one, so `reinstall_fonts` has a genuine host failure to relay.
struct FailingReinstallResources {
    calls: Rc<Cell<u32>>,
}

impl HostResources for FailingReinstallResources {
    fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        let call = self.calls.get() + 1;
        self.calls.set(call);
        if call == 1 {
            Ok(())
        } else {
            Err(CadError::Unsupported(
                "web.fonts_package_not_installed".into(),
            ))
        }
    }

    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
}

#[test]
fn reinstall_fonts_relays_the_host_refusal_verbatim() {
    let calls = Rc::new(Cell::new(0));
    let mut host_platform = platform(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    host_platform.resources = Rc::new(FailingReinstallResources {
        calls: Rc::clone(&calls),
    });
    let session = CadSession::new(options(), host_platform, RenderSessionHandle::default())
        .expect("construction install succeeds");

    assert_eq!(
        session.reinstall_fonts(),
        Err("web.fonts_package_not_installed".to_string()),
        "the host's own refusal is relayed, not replaced by a fake success"
    );
    assert_eq!(
        calls.get(),
        2,
        "the construction install plus the reinstall"
    );
}

/// A strategy that publishes its real load report only on the second call, so
/// the test can observe the report being refreshed by `reinstall_fonts`.
struct ReportingReinstallResources {
    calls: Rc<Cell<u32>>,
}

impl HostResources for ReportingReinstallResources {
    fn install_fonts(&self, core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        let call = self.calls.get() + 1;
        self.calls.set(call);
        if call > 1 {
            core.set_last_font_report(FontLoadReport {
                catalog_entries: 3,
                requested: vec!["simplex.shx".to_string(), "bigfont.shx".to_string()],
                locale: TextLocale::Latin,
                default_face: Some("simplex".into()),
                ..FontLoadReport::default()
            });
        }
        Ok(())
    }

    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
}

#[test]
fn a_successful_reinstall_refreshes_the_report_and_marks_the_guard() {
    let calls = Rc::new(Cell::new(0));
    let mut host_platform = platform(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    host_platform.resources = Rc::new(ReportingReinstallResources {
        calls: Rc::clone(&calls),
    });
    let session = CadSession::new(options(), host_platform, RenderSessionHandle::default())
        .expect("construction install succeeds");
    assert!(
        session.core.last_font_report().is_none(),
        "the first (construction) install publishes nothing"
    );

    session
        .reinstall_fonts()
        .expect("the package is now available");
    assert_eq!(calls.get(), 2);
    let answer = session.fonts().expect("the refreshed report projects");
    assert_eq!(answer.catalog_entries, 3);
    assert_eq!(answer.default_face.as_deref(), Some("simplex"));
    assert_eq!(
        answer.requested,
        vec!["simplex.shx".to_string(), "bigfont.shx".to_string()]
    );
    assert_eq!(
        session.resource_guard.get(),
        Some(session.core.request_context().document_generation),
        "the freshly installed engine belongs to this generation"
    );
}
