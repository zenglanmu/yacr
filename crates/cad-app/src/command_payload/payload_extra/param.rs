//! The `PARAM` structural op (ADR-0043 D9).
//!
//! Split out of `command_payload.rs` because that file is at the 1000-line code
//! gate. The enum's variants are re-exported through `payload_extra`, so the
//! public path is `cad_app::ParamOp` exactly as if it lived beside
//! [`crate::CommandPayload`].

/// One structural edit of the document's parameter table or one element's slot
/// bindings (ADR-0043 D9, the payload of [`crate::CommandId::Param`]).
///
/// One command performs exactly one op. Except for [`ParamOp::Config`] (a
/// runtime evaluation mode with no configuration-selection patch type), the op is
/// one transaction and one undo step.
/// Value edits deliberately do **not** live here: changing the number behind a
/// parameter is the properties command's `param.<name>` write.
pub enum ParamOp {
    /// Add a row at the next free sequence, keeping the document's names unique.
    Create {
        name: String,
        kind: cad_domain::param::ParamKind,
        expr: cad_domain::param::Expr,
    },
    /// Apply a named configuration overlay (`None` clears it).
    ///
    /// This is a runtime evaluation mode, not a stored edit: no undo step is
    /// recorded (there is no configuration-selection patch type). The derived
    /// geometry still lands in one revision/`ChangeSet`.
    Config { name: Option<String> },
    /// Remove a row that no binding and no other expression references.
    Delete { seq: cad_domain::param::ParamSeq },
    /// Change a row's name; references address the sequence, so they keep working.
    Rename {
        seq: cad_domain::param::ParamSeq,
        name: String,
    },
    /// Bind (or replace) slots on every currently selected element.
    Bind {
        slots: Vec<(String, cad_domain::param::Expr)>,
    },
    /// Remove slots from every currently selected element.
    Unbind { slots: Vec<String> },
}
