//! Headless contracts. Commands share the UI Application path.
use cad_app::{Application, Command, CommandOutcome, SessionState};
use cad_domain::*;
pub enum CliOperation {
    Scan,
    ProxyReport,
    Measure,
    ImportNotes,
    ExportNotes,
    BuildRepresentation,
    FixedViewportRender,
    Benchmark,
}
pub fn execute_command(
    application: &mut Application,
    session: &mut SessionState,
    command: Command,
) -> CadResult<CommandOutcome> {
    application.execute(session, command)
}
pub fn run_operation(_operation: CliOperation, _input: &[u8]) -> CadResult<Vec<u8>> {
    pending("cli.headless_operation")
}
