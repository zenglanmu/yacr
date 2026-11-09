//! The 2026-11-07 selection / filter / find / visibility command contracts.
//!
//! Ten AutoCAD-2026 names are implemented here. They share one rule: the
//! command layer is the authority, the session carries only derived state, and a
//! selection-only command never opens a transaction, bumps a revision or writes
//! undo history.
//!
//! ## Selection filters (`QSELECT` / `FILTER` / `GETSEL`)
//!
//! Two payload forms are accepted for `QSELECT`/`FILTER`:
//!
//! * the legacy one-shot line `PROPERTY VALUE` (also the only `GETSEL` form),
//!   carried as `CommandPayload::SelectFilter`; and
//! * the structured dialog form carried as
//!   [`crate::SelectionToolPayload::QSelect`]/`Filter` — operators, scope,
//!   apply mode and the FILTER logical tree. The structured types live in
//!   `command_payload::payload_extra`; the UI/host wiring for them is **not yet
//!   done** (the command accepts them today, the frontend does not build them).
//!
//! `PROPERTY` is `TYPE`, `LAYER`, `COLOR`, `LINETYPE`, `LINEWEIGHT`,
//! `LINETYPESCALE`/`LTSCALE`, `TRANSPARENCY`, `SPACE`, `TEXT` or `NAME`, ASCII
//! case-insensitive (the structured form also accepts the object-type and
//! operator enumerations). Any other word is an explicit refusal
//! (`qselect.unknown_property` / `filter.unknown_property`), never an empty
//! match. `VALUE` must be non-empty (except `Select All`).
//!
//! `TYPE` matches an entity's stored `object.type_key` case-insensitively and
//! also the friendly geometry-kind alias (`LINE`, `CIRCLE`, `ARC`, `POLYLINE`,
//! `ELLIPSE`, `SPLINE`, `POINT`, `TEXT`, `MTEXT`, `INSERT`, `DIMENSION`,
//! `HATCH`, `MULTILEADER`, `IMAGE`, `SHAPE`, `MESH`, `WIPEOUT`/`MASK`,
//! `OPAQUE`, `COMPOUND`). `LAYER` matches the stored layer name. The other
//! properties use the canonical `cad_app::property_value` projection
//! (`ByLayer`, `ByBlock`, `aci:N`, `#RRGGBB`, the linetype name, the canonical
//! lineweight, the transparency percentage) or read the entity's own linetype
//! scale / space / text / block name. Relative operators (`<`, `>`, `>=`,
//! `<=`) are numeric-only and refused by name on a non-numeric property.
//! `*Wildcard Match*` implements anchored ASCII-case-insensitive `*`/`?`; the
//! other AutoCAD wildcard tokens are literal (a declared boundary).
//!
//! **Scope and apply.** The dialog's apply-to is **entire drawing** or **current
//! selection**; the apply mode is **include in new selection set**, **exclude
//! from new selection set** or **append**. "Entire drawing" enumerates every
//! **top-level entity in the current space** (Autodesk does not document
//! current-space scoping; this is a declared Build choice). Nested block objects
//! are **never** selected or counted. A legacy `FILTER` is ADD/append; a legacy
//! `QSELECT` is Replace/include, with Add/append and Remove/exclude accepted.
//! A legacy payload with any other merge mode is a named refusal.
//!
//! FILTER expressions use `AND`/`OR` (≥ 1 member), `XOR` (exactly 2) and `NOT`
//! (exactly 1); groups are the typed balanced Begin/End pairs. Invalid arity is
//! a named refusal (`filter.group_arity`/`xor_arity`/`not_arity`) before any
//! read. Named filters (`filter.nfl`) are host file I/O and are refused by name
//! (`filter.named_unsupported`).
//!
//! Zero matches is **not an error**: `QSELECT` reports a `.result` diagnostic
//! and (for include) an empty selection; `FILTER` adds nothing. `FILTER` on an
//! empty current selection is valid (append to empty).
//!
//! `GETSEL` accepts only `LAYER` and `TYPE` (the AutoCAD form); every other
//! property is a named refusal there.
//!
//! ## Property similarity (`SSX` / `SELECTCOUNT`)
//!
//! The **first** selected entity is the template. Two entities are similar when
//! all five of these agree: type key (ASCII case-insensitive) plus geometry
//! discriminant, stored layer, canonical colour projection, canonical linetype
//! projection and space. `SSX` **unions** every matching model-space entity into
//! the current selection (AutoCAD's SSX adds); `SELECTCOUNT` only reports the
//! count and never touches the selection. An empty selection is refused
//! (`ssx.selection_required` / `selectcount.selection_required`), and a
//! template carrying an instance path or sub-element is refused
//! (`ssx.template_not_addressable`) because it has no single entity row to read.
//!
//! ## Text find (`-FIND` / `FIND`)
//!
//! `-FIND <needle>` is report-only: it searches model-space `AcDbText` and
//! `AcDbMText` entities (both are [`cad_domain::SemanticGeometry::Text`]) and
//! text-bearing `AcDbMultiLeader` entities for an **ASCII case-insensitive**
//! substring. It reports the match count, the entity ids and a short preview,
//! and writes nothing.
//!
//! `FIND <needle>` is the same report; `FIND <needle> => <replacement>` replaces
//! **every** occurrence in every match in **one** transaction / one undo step.
//! The replacement is a plain substring replacement: MTEXT formatting runs are
//! **not parsed**, so `\P`, `\f…;`, `{`/`}` pass through as ordinary characters.
//! An `AcDbText` target refuses a replacement containing `\r`/`\n`
//! (`text.single_line_newline`); MTEXT and MULTILEADER store line breaks as
//! `\P` through the shared `app_drawing::text_policy` rule.
//!
//! ## Visibility (`HIDEOBJECTS` / `ISOLATEOBJECTS` / `UNISOLATEOBJECTS`)
//!
//! The hidden set is **session state only** (`SessionState::hidden_entities`):
//! no database write, no revision bump, no undo entry, no persistence.
//!
//! * `HIDEOBJECTS` hides the current selection (empty selection refused) and
//!   clears the selection afterwards.
//! * `ISOLATEOBJECTS` hides every model-space entity except the current
//!   selection (empty selection refused); the selection is kept.
//! * `UNISOLATEOBJECTS` clears the hidden set; it needs no selection.
//!
//! Hidden entities are filtered by the shared visibility helper (`picking::
//! ref_visibility` callers) so they are not click/box/hover pickable, and the
//! host publishes the set as `OverlayInputs.hidden` so the renderer's
//! `FLAG_HIDDEN` discards the entity fragments. Declared boundary: the hidden
//! set is **not** removed from the resident base scene (it is fragment-suppressed
//! until a full rebuild), and the screen-space POINT-marker overlay is derived
//! from layer visibility only, so a hidden POINT's marker can still draw. A
//! hidden block member hides the addressed expansion, not every instance of the
//! containing INSERT.

use crate::command_id::CommandId;

/// The selectors the 2026-11-07 selection/filter batch adds.
///
/// Kept here as documentation of the canonical spelling; the router table in
/// `cad-ui-core` is the single authority a frontend reads.
pub const SELECT_FILTER_SELECTORS: &[(&str, CommandId)] = &[
    ("QSELECT", CommandId::QSelect),
    ("FILTER", CommandId::FilterSelect),
    ("GETSEL", CommandId::GetSelection),
    ("SSX", CommandId::SelectMatchingProperties),
    ("SELECTCOUNT", CommandId::SelectCount),
    ("-FIND", CommandId::FindReport),
    ("FIND", CommandId::FindReplace),
    ("HIDEOBJECTS", CommandId::HideObjects),
    ("ISOLATEOBJECTS", CommandId::IsolateObjects),
    ("UNISOLATEOBJECTS", CommandId::UnisolateObjects),
];
