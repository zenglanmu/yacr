//! Long-form notes for [`crate::CommandId::TextToFront`] (`TEXTTOFRONT`).
//!
//! Bring selected text, leaders and dimensions in front of every other object
//! **of their own draw-order scope**.
//!
//! # Input (declared divergence)
//!
//! Payload is [`crate::CommandPayload::None`]. The **current selection** is the
//! input — a declared divergence from AutoCAD's interactive `Select objects`
//! prompt, because this build has no in-command object-selection step. An empty
//! selection is the named `texttofront.selection` refusal; it changes nothing and
//! opens no transaction.
//!
//! # Eligibility
//!
//! A selected entity is eligible only when it is **directly addressable** (no
//! block instance, no sub-element) and its geometry is one of:
//!
//! * [`cad_domain::SemanticGeometry::Text`] (`TEXT` / `MTEXT` / dimension text is
//!   not separately selected — a `Dimension` is its own eligible kind);
//! * [`cad_domain::SemanticGeometry::MultiLeader`] (`MLEADER`, content included);
//! * [`cad_domain::SemanticGeometry::Dimension`] (any dimension subtype).
//!
//! Every other selected object is **not moved** and does not fail the command: it
//! produces one per-entity `texttofront.skipped` diagnostic. A ref with an
//! instance path or sub-element, a ref from another document and a ref for a
//! missing entity are likewise reported and skipped. Duplicate refs collapse to
//! one entity.
//!
//! # Scopes and ordering rule
//!
//! The reorder is planned **per `SpaceId`** — model space, each paper layout and
//! each block definition — exactly like `HATCHTOBACK` and `DRAWORDER`: a layout's
//! draw order is only meaningful against that layout's rows, and a block
//! definition's members only against each other.
//!
//! For every scope, let `base` be the current maximum `draw_order` over that
//! scope's **non-eligible** entities (`0` when there is none). The eligible
//! selected entities of that scope are sorted by `(draw_order, entity id)` —
//! lowest first, so their relative order is preserved — and receive the
//! contiguous orders `base + 1 ..= base + n`. A scope whose eligible objects are
//! all already above its `base` keeps its exact orders (never compacted).
//!
//! Only rows whose order really changes are staged. When no scope changes, the
//! command returns `texttofront.unchanged` with **no** transaction, no revision
//! bump and no undo entry. An `i64` overflow in any scope's target orders is the
//! explicit `texttofront.order_exhausted` refusal and nothing is written.
//!
//! The write is one transaction / one undo step with full before/after entity
//! patches (`update_entity`), so undo restores each entity's exact prior order.
//!
//! # Declared limits
//!
//! * Selected **non-text** objects are never moved; only eligible objects are.
//! * AutoCAD's **all text in the drawing** form (and its Draw Order dialog) is
//!   **NOT IMPLEMENTED**: the payload carries no "all text" option, so there is no
//!   channel to request it; only the explicit selection is acted on.
//! * There is no draw-order preview and no `Bring Above Object` /
//!   `Bring Under Object` variant; attributes and tables are skipped by the
//!   eligibility rule.
//!
//! Work-only: Viewer mode refuses the command at the command layer.
