//! `OUTERSHELL` on the host boundary: the command is admitted and its selection
//! rule is the command's own named refusal (the geometry contract lives in
//! `cad-app`). No GUI or GPU is involved.
use super::*;

#[test]
fn the_outershell_command_is_admitted_and_names_its_selection_rule() {
    let core = core();
    let error = core
        .command(CommandId::OuterShell, CommandPayload::None)
        .expect_err("the demo selection is empty");
    let text = match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    };
    assert!(text.contains("outershell.needs_two"), "{text}");
    assert!(
        core.controller.borrow().drawing().unwrap().revision().0 == 0,
        "a refused shell writes nothing"
    );
}
