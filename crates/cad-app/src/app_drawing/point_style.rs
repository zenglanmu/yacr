//! The drawing-global POINT display style commands (`PDMODE <n>` / `PDSIZE <n>`).
//!
//! These are the two command-line forms of AutoCAD's `$PDMODE` / `$PDSIZE` system
//! variables: each takes **one numeric argument** and writes one header value of
//! the open drawing, so the style that used to arrive only at import is settable
//! from the command line.
//!
//! Contract:
//!
//! * **one value rule, in the database**: the write goes through
//!   [`cad_db::DrawingDatabase::set_point_display_style`], which stores the mode
//!   and the size **raw** (the display layer is the single interpreter of the
//!   mode bits, the size sign and the sanitisation rule) and refuses a non-finite
//!   size. The command layer adds no clamp, no rounding and no default: an
//!   out-of-grammar argument is refused by name before any candidate exists.
//! * **the display refreshes through the revision**: a real write publishes the
//!   database's `ChangeSet` (one revision bump, no entity changes) and the
//!   document is replaced with the mutated candidate — the ordinary "publish a
//!   new `Arc<DrawingDatabase>`" path the layer-table writes use. The render
//!   scene derives the POINT display style per **base revision**
//!   (`cad_app::render_scene`), so the new revision is exactly what invalidates
//!   the cached marker style; nothing hand-rolls a revision write.
//! * **no history entry**: `cad-history` stores entity patches only, so there is
//!   no header-variable patch kind and no undo step is recorded. That is a
//!   deliberate, documented exception to "one command = one undo step": faking a
//!   patch would make UNDO claim to restore a value it never stored.
//! * **Work-only**: both commands are document mutations, so Viewer mode refuses
//!   them through the existing rule
//!   ([`CommandId::requires_work_mode`](crate::CommandId::requires_work_mode) +
//!   [`SessionState::authorize`](crate::SessionState::authorize) →
//!   `CadError::PermissionDenied`), the same rule every layer/database write uses.
//! * **the DXF exporter writes both values** (the acadrust export path in
//!   `cad_import_acadrust::export`), so a SAVE→OPEN cycle keeps the style: this is
//!   the acadrust writer's `$PDMODE` / `$PDSIZE` header pair plus our own reader,
//!   i.e. **internal consistency**, not a third-party-vendor compatibility claim.

use super::*;

impl Application {
    /// Dispatch `PDMODE <n>` / `PDSIZE <n>`: write one raw header value.
    pub(crate) fn point_display_command(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        // Resolve the requested pair from the stored pair plus the one value this
        // command carries. A wrong payload is refused before any candidate exists.
        let (mode, size, code, message) = match &command.payload {
            CommandPayload::PointDisplayMode(mode) => (
                *mode,
                drawing.point_display_size(),
                "drawing.point_display_mode",
                format!("point display mode: {mode} ($PDMODE)"),
            ),
            CommandPayload::PointDisplaySize(size) => {
                if !size.is_finite() {
                    return Err(CadError::InvalidInput(format!(
                        "PDSIZE needs a finite size, got {size}"
                    )));
                }
                (
                    drawing.point_display_mode(),
                    *size,
                    "drawing.point_display_size",
                    format!("point display size: {size} ($PDSIZE)"),
                )
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "PDMODE/PDSIZE need one numeric argument".into(),
                ))
            }
        };

        let mut candidate = drawing.as_ref().clone();
        let changeset = candidate.set_point_display_style(
            mode,
            size,
            next_transaction_id(),
            "set point display style",
        )?;
        // A write of the stored pair is an authoritative no-op: the revision did
        // not move, so the report says so instead of claiming a change.
        let changed = changeset.before != changeset.after;

        // Publish the already-validated candidate. No history record: there is no
        // header-variable patch kind (see the module docs).
        self.workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::Invariant("document disappeared while publishing".into()))?
            .drawing = Arc::new(candidate);

        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(changeset)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: code.into(),
                message: if changed {
                    message
                } else {
                    format!("{message} (already stored, no change)")
                },
            }],
            measurement: None,
            point_report: None,
        })
    }
}
