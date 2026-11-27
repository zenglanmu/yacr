//! `INSERT <name>` / `BLOCK <name>`: start the capture with a pending block name.
//!
//! The bare selectors are point-then-name captures. The typed form supplies the
//! name up front: the session starts the same capture and records the name as
//! pending text, which is applied as the terminal text as soon as the point is
//! captured (one click then places/defines the named block). The create command
//! still owns every semantic refusal.

use cad_app::DrawToolKind;

use super::commands::{
    invalid_block_name_argument_reason, parse_block_name_argument, BLOCK_SELECTOR, INSERT_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `INSERT <name>` line.
    pub(super) fn insert_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        self.block_capture_argument(INSERT_SELECTOR, DrawToolKind::InsertBlock, argument, text)
    }

    /// Apply one typed `BLOCK <name>` line.
    pub(super) fn block_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        self.block_capture_argument(BLOCK_SELECTOR, DrawToolKind::BlockDefine, argument, text)
    }

    fn block_capture_argument(
        &self,
        selector: &str,
        kind: DrawToolKind,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let Some(name) = parse_block_name_argument(argument) else {
            return self.refuse_argument(invalid_block_name_argument_reason(selector, text));
        };
        self.submit_interactions(vec![cad_app::interaction::InteractionRequest::BeginDraw(
            kind,
        )])?;
        // `begin_draw` clears any stale pending text, so record the chosen name
        // only after the capture really started.
        self.core
            .controller
            .borrow_mut()
            .session
            .set_pending_tool_text(Some(name));
        Ok(())
    }
}
