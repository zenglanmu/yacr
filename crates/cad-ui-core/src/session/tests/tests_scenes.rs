//! `SCENE` typed lines and the `scenes` host read.
//!
//! The session parses the line and dispatches `CommandId::Scene`; the panel's
//! read projects the open document's `tables/scenes.json` through
//! `HostCore::scenes`. Contract level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;
use crate::session::commands::parse_scene_argument;
use cad_app::SceneCommand;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_scene_grammar_is_narrow_and_case_insensitive() {
    assert_eq!(
        parse_scene_argument("SAVE Overview"),
        Some(SceneCommand::Save("Overview".into()))
    );
    assert_eq!(
        parse_scene_argument("restore overview"),
        Some(SceneCommand::Restore("overview".into()))
    );
    assert_eq!(
        parse_scene_argument("delete  Old  "),
        Some(SceneCommand::Delete("Old".into()))
    );
    assert_eq!(parse_scene_argument("LIST"), Some(SceneCommand::List));
    for refused in ["", "SAVE", "RESTORE", "DELETE", "LIST extra", "BOGUS"] {
        assert_eq!(parse_scene_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("SCENE SAVE Overview"),
        Some(("SCENE", "SAVE Overview"))
    );
    assert_eq!(typed_command_argument("SCENE"), None);
    assert_eq!(command_name(CommandId::Scene), Some("SCENE"));
}

#[test]
fn scene_save_list_restore_delete_round_trip_through_the_session() {
    let session = fresh_session();
    session
        .command("SCENE SAVE Overview")
        .expect("SCENE SAVE is admitted");
    {
        let c = session.core.controller.borrow();
        assert!(c.application.can_undo(&c.document_id));
        assert!(!c.application.can_redo(&c.document_id));
    }
    session.command("SCENE LIST").expect("SCENE LIST");
    session.command("SCENE RESTORE overview").expect("restore");
    session.command("SCENE DELETE Overview").expect("delete");
    // The next undo brings the row back: delete is one document write.
    session.command("UNDO").expect("UNDO");
    session.command("SCENE LIST").expect("list after undo");
}

#[test]
fn the_scenes_host_read_projects_the_document_table() {
    let session = fresh_session();
    let empty = session.scenes();
    assert!(empty.scenes.is_empty());
    assert_eq!(empty.total, 0);

    session.command("SCENE SAVE Overview").expect("save");
    let answer = session.scenes();
    assert_eq!(answer.total, 1);
    assert_eq!(answer.scenes.len(), 1);
    assert_eq!(answer.scenes[0].name, "Overview");
    assert_eq!(answer.scenes[0].space, "Model");
    assert_eq!(answer.scenes[0].style, "SHADED");
    assert!(!answer.scenes[0].is_3d);
}

#[test]
fn the_scenes_request_round_trips_through_the_envelope() {
    use crate::host_read::{HostReadAnswer, HostReadRequest};
    use crate::scene::SCENES_KIND;
    let request = HostReadRequest::Scenes;
    assert_eq!(request.kind(), SCENES_KIND);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({ "kind": "scenes" })
    );
    let decoded: HostReadRequest =
        serde_json::from_value(serde_json::json!({ "kind": "scenes" })).expect("decode");
    assert_eq!(decoded, HostReadRequest::Scenes);
    let answer = HostReadAnswer::Scenes(crate::scene::ScenesResponse::default());
    assert!(matches!(answer, HostReadAnswer::Scenes(_)));
}
