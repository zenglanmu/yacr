//! `TEXTTOFRONT` (AutoCAD 2026): bring selected text, leaders and dimensions in
//! front of every other object.
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
//! A selected entity is eligible only when it is **directly addressable model
//! space** and its geometry is one of:
//!
//! * [`cad_domain::SemanticGeometry::Text`] (`TEXT` / `MTEXT` / dimension text is
//!   not separately selected — a `Dimension` is its own eligible kind);
//! * [`cad_domain::SemanticGeometry::MultiLeader`] (`MLEADER`, content included);
//! * [`cad_domain::SemanticGeometry::Dimension`] (any dimension subtype).
//!
//! Every other selected object is **not moved** and does not fail the command: it
//! produces one per-entity `texttofront.skipped` diagnostic. A ref with an
//! instance path or sub-element, a ref from another document, a ref for a missing
//! entity and a ref outside model space are likewise reported and skipped, never
//! reached into. Duplicate refs collapse to one entity.
//!
//! # Ordering rule
//!
//! Let `base` be the current maximum `draw_order` over **all model-space
//! non-eligible entities** (`0` when there is none). The eligible selected
//! entities are sorted by `(draw_order, entity id)` — lowest first, so their
//! relative order is preserved — and receive the contiguous orders
//! `base + 1 ..= base + n`.
//!
//! Only rows whose order really changes are staged. When every eligible entity is
//! already above `base`, the command returns `texttofront.unchanged` with **no
//! transaction**, no revision bump and no undo entry. An `i64` overflow in the
//! target orders is the explicit `texttofront.order_exhausted` refusal.
//!
//! The write is one transaction / one undo step with full before/after entity
//! patches (`update_entity`), so undo restores each entity's exact prior order.
//!
//! # Scope and declared limits
//!
//! * **Model space only**: paper-space and block-definition draw order are not
//!   reordered (this build has no paper-space draw-order model).
//! * Selected **non-text** objects are never moved; only eligible objects are.
//! * AutoCAD's **all text in the drawing** form (and its Draw Order dialog) is
//!   **NOT IMPLEMENTED**: v1 only acts on the explicit selection.
//! * There is no draw-order preview, no `Bring Above Object` / `Bring Under
//!   Object` variant and no `Texttofront` of attributes/tables — those kinds are
//!   skipped by the eligibility rule.
//!
//! Work-only: Viewer mode refuses the command at the command layer.
