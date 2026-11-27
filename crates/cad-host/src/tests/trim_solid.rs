//! `TRIMSOLID` on the host boundary: the command is admitted and its selection
//! rule is the command's own named refusal. The trim/keep-cutter contract lives
//! in `cad-app`. No GUI or GPU is involved.
use super::*;

#[test]
fn the_trimsolid_command_is_admitted_and_names_its_selection_rule() {
    let core = core();
    let error = core
        .command(CommandId::TrimSolid, CommandPayload::None)
        .expect_err("the demo selection is empty");
    let text = match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    };
    assert!(text.contains("trimsolid.needs_two"), "{text}");
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision().0,
        0,
        "a refused trim writes nothing"
    );
}
