//! Long-form notes for [`crate::CommandId::MatchProp`] (`MATCHPROP`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here. Nothing here is
//! implemented in this module — the property transfer lives in
//! `app_drawing::match_prop`.
//!
//! `MATCHPROP` paints the display properties of the current selection's
//! **first** entity onto every other selected entity — the selection-based
//! form of AutoCAD's property painter.
//!
//! Payload is [`crate::CommandPayload::None`]: the selection is the input,
//! read from the session at apply time. The selection's **own order** (the
//! pick order [`crate::SelectionSet`] preserves, never re-sorted by id) is
//! the contract: the first selected entity is the **source**, every other
//! selected entity is a **destination**. Fewer than two selected entities is
//! an explicit refusal (`matchprop.needs_two`) — there is no
//! source-without-destination form.
//!
//! Exactly the five writable display properties are transferred: `layer`,
//! `color`, `linetype`, `lineweight`, `transparency` (the set
//! [`crate::property_value::SUPPORTED_KEYS`] accepts, projected and
//! re-parsed through that module's one grammar). **Not transferred** — and
//! never silently approximated: MATCHPROP's per-property *Settings* subset
//! (which properties to copy), text style/height, dimension style, hatch
//! pattern and every other non-display value, and the entity's own linetype
//! scale (not one of the five keys, so a named destination linetype keeps the
//! destination's scale). Source and destination **kinds may differ** — that
//! is the point — as long as every value is valid for the destination.
//!
//! Every refusal happens before any mutation (zero database change, zero
//! undo entry): `matchprop.needs_two`; `property.not_addressable` /
//! `property.target_missing` for a source **or** destination ref that is a
//! block-instance member, a sub-element, another document's, or missing (the
//! property-write rule, reused); and `matchprop.source_value` naming the
//! source entity and the property when the source's stored value has no
//! writable form (the parser's own `property.*` code is preserved).
//!
//! A destination that already matches contributes no change and is not
//! staged. If **every** destination already matches, the command reports
//! `matchprop.unchanged` with **no** transaction and no empty undo entry;
//! otherwise all changed entities share one transaction / one undo step.
//! Work-only.
