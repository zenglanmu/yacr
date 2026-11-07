//! Long-form notes for [`crate::CommandId::Overkill`] (`OVERKILL`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant keeps a compact doc and links here. The implementation is
//! `app_drawing::modify_extra::overkill`.
//!
//! ## Input
//!
//! Payload is [`crate::CommandPayload::None`]; the current selection is the
//! whole input (the declared divergence from AutoCAD's interactive
//! select-objects prompt). An empty selection is `overkill.selection`. A ref
//! that is not directly addressable (another document, a block-instance member,
//! a sub-element) is `overkill.not_addressable`; a missing entity is
//! `overkill.target_missing`. Both refuse the whole command and write nothing.
//!
//! ## What v1 removes
//!
//! **Exact duplicates only.** Two selected entities are duplicates when they are
//! directly addressable, in the **same space** and on the **same layer**, have
//! the same geometry kind and compare equal within `1e-9` world units:
//!
//! | kind | comparison |
//! | --- | --- |
//! | LINE | the unordered endpoint pair |
//! | LWPOLYLINE | points pairwise, per-segment bulges within `1e-9`, closed flag |
//! | CIRCLE | centre, normal, radius |
//! | ARC | centre, normal, radius, start, sweep |
//! | SPLINE | degree, knots, control points, weights |
//!
//! Any other kind (and any kind paired with a different kind) is never a
//! duplicate. Each group keeps its lowest `draw_order` (ties: lowest entity id);
//! the rest are deleted in **one transaction / one undo step** through the shared
//! delete path, so undo restores them with their render attributes and
//! visibility membership.
//!
//! ## Declared NOT IMPLEMENTED
//!
//! Everything AutoCAD's OVERKILL does beyond exact duplicates: partial overlaps,
//! collinear merging, **arc-over-circle replacement**, polyline segment
//! optimization, the numeric tolerance option, and the ignore-layer/colour
//! options. The success and no-op paths both report
//! `overkill.v1_exact_duplicates_only` as an informational diagnostic, and the
//! command never merges or deletes across layers or spaces.
//!
//! Success is `overkill.applied` with the deleted count; a selection with no
//! duplicates creates **no** transaction and reports `overkill.unchanged`.
//! Work-only.
