//! Long-form notes for [`crate::CommandId::GripStretch`] (`GRIPSTRETCH`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here.
//!
//! ## Input
//!
//! Payload is [`crate::CommandPayload::GripStretch`] (boxed
//! [`crate::GripStretchCommand`] `{ id, to }`, where `id` is a
//! [`crate::grips::GripId`]). It is the **payload path of a pointer gesture**:
//! `cad_host` produces it, and only from the release of an armed grip drag — its
//! pointer routing aims at a handle of the current selection at press time,
//! tracks the drag and dispatches exactly one `GripStretch` on release (a drag
//! that did not move the handle dispatches nothing, and a cancel drops the armed
//! state without writing). This crate keeps no grip gesture state and never
//! fabricates one; 3D grip editing is NOT IMPLEMENTED, and this doc records the
//! code path, not a runtime or visual acceptance. The bare `GRIPSTRETCH`
//! selector carries no handle, so it is refused explicitly (no fabricated drag).
//!
//! ## Refusals and history
//!
//! The handler re-reads `id.entity` (a missing entity is `grip.target_missing`
//! and a non-model-space one reuses `property.not_addressable`), then
//! re-validates the handle against the current geometry via
//! [`crate::grips::apply_grip`]. Every refusal — `grip.stale_handle`,
//! `grip.zero_displacement`, `grip.unsupported_kind`,
//! `grip.non_finite_target` — happens **before** the transaction is opened, so
//! nothing is written and no empty undo entry exists. It records history (unlike
//! the layer-table writes) and never routes through
//! [`crate::CommandId::StretchEntities`]. Work-only.
