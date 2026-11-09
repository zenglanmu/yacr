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
//! Payload is [`crate::CommandPayload::None`] (all modelled properties) or the
//! typed [`crate::MatchPropSettings`] subset. The selection is the object input,
//! read from the session at apply time. The selection's **own order** (the
//! pick order [`crate::SelectionSet`] preserves, never re-sorted by id) is
//! the contract: the first selected entity is the **source**, every other
//! selected entity is a **destination**. Fewer than two selected entities is
//! an explicit refusal (`matchprop.needs_two`) — there is no
//! source-without-destination form.
//!
//! Exactly the six modelled Basic properties are transferable — `layer`,
//! `color`, `linetype`, `lineweight`, `transparency` (the set
//! [`crate::property_value::SUPPORTED_KEYS`] accepts, projected and
//! re-parsed through that module's one grammar) — plus the entity's own
//! **linetype scale**. The three **Special** groups are also modelled: text
//! style (TEXT/MTEXT, never height), dimension style (never overrides) and
//! hatch properties (never the origin). [`crate::MatchPropSettings`] gates the
//! Basic group and each Special group independently; a disabled property is
//! neither resolved nor written, and a destination of another object class
//! skips a Special group instead of failing the command. Plot style and
//! thickness are not modelled on an entity (remaining boundary). Source and
//! destination **kinds may differ** — that is the point — as long as every
//! enabled value is valid for the destination.
//!
//! Every refusal happens before any mutation (zero database change, zero
//! undo entry): `matchprop.needs_two`; `matchprop.invalid_payload`;
//! `property.not_addressable` / `property.target_missing` for a source **or**
//! destination ref that is a block-instance member, a sub-element, another
//! document's, or missing (the property-write rule, reused); `layer.locked` for
//! a destination on, or moving onto, a locked layer; and `matchprop.source_value`
//! naming the source entity and the property when an enabled source value has
//! no writable form (the parser's own `property.*` code is preserved).
//!
//! A destination that already matches contributes no change and is not
//! staged. If **every** destination already matches, the command reports
//! `matchprop.unchanged` with **no** transaction and no empty undo entry;
//! otherwise all changed entities share one transaction / one undo step.
//! Work-only.
