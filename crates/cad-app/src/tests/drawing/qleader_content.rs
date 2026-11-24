//! `QLEADER` / `LEADER`: the legacy spellings create the same MULTILEADER
//! content through the shared creation path.
//!
//! Split out of `mleader_content.rs` for the code-size gate. Each test pins one
//! of the legacy creation options this build adds on top of the `MLEADER`
//! payload: `CONTENT MTEXT|BLOCK <name>|TOLERANCE`, `QLEADER`'s `ARROW ON|OFF`
//! (the created line's own arrowhead override) and `MAXPOINTS` (the two-vertex
//! creation bound). Refusals are the shared `mleader.*` ones (or the selector's
//! own `qleader.invalid_max_points`) and write nothing.

use super::mleader_content::{block_app, round_trip, the_leader};
use super::*;

fn legacy_leader_request(
    content: Option<MLeaderContentSpec>,
    text: &str,
    arrowhead: Option<bool>,
    max_points: Option<u32>,
) -> CommandPayload {
    CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Leader(LeaderCommand {
        arrow: p3(0.0, 0.0, 0.0),
        landing: p3(10.0, 2.0, 0.0),
        text: text.to_string(),
        content,
        arrowhead,
        max_points,
    })))
}

#[test]
fn qleader_content_block_and_tolerance_use_the_shared_domain_rules() {
    // Block content: the very same resolution, stored `172 = 1`, and DXF round trip.
    let (mut app, mut session) = block_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::QLeader,
        legacy_leader_request(
            Some(MLeaderContentSpec::Block("Door".into())),
            "",
            None,
            None,
        ),
    )
    .expect("QLEADER CONTENT BLOCK creates through the MLEADER path");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert!(app.can_undo(&DocumentId(1)), "one undo step");
    let leader = the_leader(&app);
    assert_eq!(leader.content().kind(), MLeaderContentKind::Block);
    assert_eq!(leader.block_content().expect("block payload").name, "Door");
    let reimported = round_trip(&app);
    assert_eq!(reimported.content().kind(), MLeaderContentKind::Block);

    // Tolerance content: the `|`-separated frame, stored `172 = 3`, and round trip.
    let (mut app, mut session) = block_app();
    execute(
        &mut app,
        &mut session,
        CommandId::QLeader,
        legacy_leader_request(Some(MLeaderContentSpec::Tolerance), "|a|b|c", None, None),
    )
    .expect("QLEADER CONTENT TOLERANCE creates through the MLEADER path");
    let leader = the_leader(&app);
    assert_eq!(leader.content().kind(), MLeaderContentKind::Tolerance);
    assert_eq!(
        leader.tolerance_content().expect("frame payload").text,
        "|a|b|c"
    );
    let reimported = round_trip(&app);
    assert_eq!(reimported.content().kind(), MLeaderContentKind::Tolerance);

    // LEADER takes the same content prefix.
    let (mut app, mut session) = block_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Leader,
        legacy_leader_request(
            Some(MLeaderContentSpec::Block("Door".into())),
            "",
            None,
            None,
        ),
    )
    .expect("LEADER CONTENT BLOCK creates the same leader");
    assert_eq!(the_leader(&app).content().kind(), MLeaderContentKind::Block);

    // A block-content line with annotation text is the shared named refusal and
    // writes nothing.
    let (mut app, mut session) = block_app();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QLeader,
        legacy_leader_request(
            Some(MLeaderContentSpec::Block("Door".into())),
            "unexpected",
            None,
            None,
        ),
    )
    .expect_err("block content takes no text");
    assert!(
        format!("{error}").contains("mleader.block_text_unexpected"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .is_empty());
}

#[test]
fn qleader_arrow_off_is_a_line_override_that_round_trips() {
    // `ARROW OFF`: line 40 = 0 with the per-line override bit, so a reload reads
    // "no arrowhead" from the entity even though the style draws one.
    let (mut app, mut session) = block_app();
    execute(
        &mut app,
        &mut session,
        CommandId::QLeader,
        legacy_leader_request(None, "Off", Some(false), None),
    )
    .expect("QLEADER ARROW OFF");
    let leader = the_leader(&app);
    let line = &leader.lines()[0];
    assert_eq!(line.arrowhead_size, 0.0);
    assert!(
        line.override_flags & cad_domain::MLeaderLine::ARROWHEAD_SIZE != 0,
        "the per-line arrowhead override bit is set"
    );
    let reimported = round_trip(&app);
    let reimported_line = &reimported.lines()[0];
    assert_eq!(reimported_line.arrowhead_size, 0.0);
    assert!(
        reimported_line.override_flags & cad_domain::MLeaderLine::ARROWHEAD_SIZE != 0,
        "the override bit survives DXF"
    );

    // `ARROW ON` (and no option) draws the style arrowhead and sets no override.
    for arrowhead in [None, Some(true)] {
        let (mut app, mut session) = block_app();
        execute(
            &mut app,
            &mut session,
            CommandId::QLeader,
            legacy_leader_request(None, "On", arrowhead, None),
        )
        .expect("QLEADER ARROW ON");
        let leader = the_leader(&app);
        let line = &leader.lines()[0];
        assert_eq!(line.arrowhead_size, leader.style().arrowhead_size);
        assert_eq!(
            line.override_flags & cad_domain::MLeaderLine::ARROWHEAD_SIZE,
            0
        );
    }
}

#[test]
fn qleader_max_points_is_honoured_or_refused_by_name_without_writing() {
    // The creation has two vertices: `MAXPOINTS 2` is representable and creates.
    let (mut app, mut session) = block_app();
    execute(
        &mut app,
        &mut session,
        CommandId::QLeader,
        legacy_leader_request(None, "Two", None, Some(2)),
    )
    .expect("MAXPOINTS 2 matches the created line");

    // A higher request is the shared named refusal; the drawing is untouched.
    let (mut app, mut session) = block_app();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QLeader,
        legacy_leader_request(None, "Three", None, Some(3)),
    )
    .expect_err("three vertices cannot be created");
    assert!(
        format!("{error}").contains("mleader.option_max_points"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .is_empty());

    // Below the minimum is the selector's own named refusal.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QLeader,
        legacy_leader_request(None, "One", None, Some(1)),
    )
    .expect_err("one vertex cannot be a leader");
    assert!(
        format!("{error}").contains("qleader.invalid_max_points"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}
