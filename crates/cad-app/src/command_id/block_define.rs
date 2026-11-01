//! Long-form notes for [`crate::CommandId::DefineBlock`] (`BLOCK`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here. The command
//! body lives in `app_drawing::block_define`.
//!
//! ## Input
//!
//! Payload is [`crate::CommandPayload::DefineBlock`] `{ refs, base, name }`:
//!
//! * `refs` is the selection snapshot taken when the interactive tool started
//!   ([`crate::draw_tool::DrawToolKind::BlockDefine`]). An empty list is
//!   `block.selection_empty`.
//! * `base` is the one picked base point. A non-finite coordinate is
//!   `block.invalid_point`.
//! * `name` is the terminal typed string. It is trimmed; an empty/whitespace-only
//!   name, a `\r`/`\n`, more than
//!   [`crate::draw_tool::MAX_TOOL_TEXT_CHARS`] characters, or any
//!   AutoCAD-illegal character (`<>/\":;?*|=`'`) is `block.invalid_name`. A name
//!   that already resolves exact-first or ASCII case-insensitively is
//!   `block.duplicate_name`.
//!
//! Each ref must be directly addressable (`block.not_addressable` for an
//! instance path or sub-element), resolve in this document
//! (`block.target_missing` for another document's id or a missing entity), and
//! live in model space (`block.not_model_space` for paper space or a block
//! member).
//!
//! ## Block-space convention (do not translate members)
//!
//! The selected entities keep their geometry **bit-for-bit** in their current
//! model-space coordinates. Only their `space` changes to
//! [`cad_domain::SpaceId::Block`]`(new_id)`; the command never subtracts the
//! base point from member geometry.
//!
//! The new definition stores `BlockMetadata { name, base_point: base }` with the
//! picked base. The reference the command inserts uses the **same composition as
//! `INSERT`** with `insertion = base` and `base_point = base`, i.e.
//! `T(base) · T(−base)` — the identity transform. A later `INSERT` at `I` uses
//! `T(I) · T(−base)`, so the picked base maps to `I`; translating the members by
//! `−base` here would double-subtract and place them at `I − base`. The command
//! therefore asserts that every member's world position is unchanged.
//!
//! ## One transaction, one undo step
//!
//! Every refusal happens before `begin_drawing_transaction`, so a refused
//! `BLOCK` writes nothing: no revision bump, no `ChangeSet`, no undo entry. A
//! locked member layer or a locked active layer is refused by the stager's own
//! `layer.locked` chokepoint before any mutation.
//!
//! On success the definition row, the member space changes and the reference
//! insert commit as **one revision**, and the history record carries one
//! [`cad_db::BlockPatch`] plus one entity patch per member and the reference.
//! The undo/redo path replays both batches atomically (`merge_key: None`), so a
//! `BLOCK` is exactly one undo step.
//!
//! The reference is inserted on the session's active layer in model space with
//! `draw_order = max(member draw_order)`; an unselected entity with a higher
//! draw order therefore still draws above it. On success the session selection
//! is cleared, because those ids are now block members rather than selectable
//! model-space objects.
//!
//! Work-only: it writes the drawing and the block table, so Viewer mode refuses
//! it through [`crate::CommandId::requires_work_mode`] +
//! [`crate::SessionState::authorize`] (`CadError::PermissionDenied`).
