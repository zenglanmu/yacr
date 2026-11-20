//! Host-owned file command dispatch: OPEN / NEW / CLOSE / PLOT / SAVE / EXPORT.
//!
//! These commands are performed by the platform (a file chooser, a download, an
//! archive/connector write), never by the application command layer, so they
//! cannot ride the ordinary traced app entry. Each one records its **real host
//! outcome** as the command trace; an async Open success means the delivery
//! STARTED, not that the import finished.
//!
//! Split out of `lib.rs` so the dispatch function stays inside the repository's
//! 1000-line file ceiling. The helper answers `None` for every command it does
//! not own, and the caller then runs the app funnel.
use super::*;

impl HostCore {
    /// Dispatch one host-owned file command, or `None` when `command_id` is not
    /// one of them.
    ///
    /// `SwitchBackend` is included because its refusal never executes anything:
    /// it records no command trace, unlike the file funnels above it.
    pub(crate) fn dispatch_host_owned(
        &self,
        command_id: CommandId,
        none: &Arc<CommandOutcome>,
    ) -> Option<CadResult<Option<Arc<CommandOutcome>>>> {
        let record = |result: CadResult<()>| match result {
            Ok(()) => {
                self.record_raw_trace(command_id, Ok(Arc::clone(none)));
                Ok(None)
            }
            Err(error) => {
                self.record_raw_trace(command_id, Err(error.clone()));
                Err(error)
            }
        };
        Some(match command_id {
            CommandId::OpenDrawing => record(self.request_open()),
            CommandId::NewDrawing => record(self.request_new()),
            CommandId::CloseDrawing => record(self.request_close()),
            CommandId::PlotDrawing => record(self.request_plot()),
            CommandId::SaveDrawingAs => record(self.request_save()),
            CommandId::Export => record(self.request_export()),
            // A refusal that never executed records no command trace.
            CommandId::SwitchBackend => Err(CadError::Unsupported(
                "backend switching is not available through this host".into(),
            )),
            _ => return None,
        })
    }
}
