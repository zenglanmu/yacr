//! The typed `SPACE <argument>` line: layout-tab switching without a new IPC.
//!
//! A frontend tab click sends one ordinary command-line text — `SPACE MODEL`,
//! `SPACE #2` or `SPACE Sheet One` — through the existing `cad_command` funnel.
//! The argument is resolved against the **authoritative drawing**; an unknown or
//! ambiguous name is refused by name instead of dispatching a guessed layout id.
//! The payload is the same [`cad_app::CommandPayload::Space`] the IPC path
//! already uses, so there is exactly one space-switch command.

use cad_app::{CommandId, CommandPayload};
use cad_domain::{LayoutId, SpaceId};

use super::{CadSession, SPACE_SELECTOR};

impl CadSession {
    /// Dispatch one typed `SPACE <argument>` line.
    ///
    /// | argument | behaviour |
    /// | --- | --- |
    /// | `MODEL` (case-insensitive) | [`SpaceId::Model`] |
    /// | `#<u128>` | [`SpaceId::Paper`] with that `LayoutId` |
    /// | a layout display name | exact match first, then a **unique** case-insensitive match |
    /// | anything else | `unknown layout: <argument>` / `ambiguous layout: <argument>` |
    ///
    /// The whole remainder of the line is the name, so `SPACE Sheet One` works.
    /// The admitted selector name (never the typed value) is echoed in the
    /// command log; nothing is dispatched when resolution fails.
    pub(super) fn space_argument(&self, argument: &str) -> Result<(), String> {
        let space = match self.resolve_space_argument(argument) {
            Ok(space) => space,
            Err(reason) => return self.refuse_argument(reason),
        };
        // Echo the admitted selector name only, exactly like the other typed
        // argument selectors.
        self.core.record_command(SPACE_SELECTOR);
        self.dispatch_payload(CommandId::SwitchSpace, CommandPayload::Space(space))
    }

    /// Resolve the argument to a real space, refusing rather than guessing.
    fn resolve_space_argument(&self, argument: &str) -> Result<SpaceId, String> {
        if argument.eq_ignore_ascii_case("MODEL") {
            return Ok(SpaceId::Model);
        }
        let numeric = argument.strip_prefix('#').filter(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        });
        if let Some(id) = numeric.and_then(|digits| digits.parse::<u128>().ok()) {
            return Ok(SpaceId::Paper(LayoutId(id)));
        }
        self.resolve_layout_name(argument)
    }

    /// Resolve a layout display name against the authoritative drawing.
    ///
    /// Exact match wins (so two layouts differing only by case stay addressable
    /// by their exact spelling); otherwise a **single** case-insensitive match
    /// is accepted. Zero matches and multiple case-insensitive matches are
    /// named refusals, never a first-match guess.
    fn resolve_layout_name(&self, argument: &str) -> Result<SpaceId, String> {
        let unknown = || format!("unknown layout: {argument}");
        let controller = self.core.controller.borrow();
        let Some(drawing) = controller.drawing() else {
            return Err(unknown());
        };
        let descriptors: Vec<_> = cad_app::render_scene::layout_descriptors(drawing.as_ref())
            .into_iter()
            // Model space is its own tab and is never selectable by a paper
            // layout name.
            .filter(|descriptor| descriptor.id.0 != 0)
            .collect();
        if let Some(descriptor) = descriptors
            .iter()
            .find(|descriptor| descriptor.name == argument)
        {
            return Ok(SpaceId::Paper(descriptor.id));
        }
        let matches: Vec<_> = descriptors
            .iter()
            .filter(|descriptor| descriptor.name.eq_ignore_ascii_case(argument))
            .collect();
        match matches.as_slice() {
            [descriptor] => Ok(SpaceId::Paper(descriptor.id)),
            [] => Err(unknown()),
            _ => Err(format!("ambiguous layout: {argument}")),
        }
    }
}
