//! The 2026-11-07 selection / filter / find / visibility command contracts.
//!
//! Ten AutoCAD-2026 names are implemented here. They share one rule: the
//! command layer is the authority, the session carries only derived state, and a
//! selection-only command never opens a transaction, bumps a revision or writes
//! undo history.
//!
//! ## Selection filters (`QSELECT` / `FILTER` / `GETSEL`)
//!
//! Grammar, parsed from the typed argument (`QSELECT <PROPERTY> <VALUE>`):
//!
//! * `PROPERTY` is `TYPE`, `LAYER`, `COLOR` or `LINETYPE`, ASCII
//!   case-insensitive. Any other word is an explicit refusal
//!   (`qselect.unknown_property` / `filter.unknown_property` /
//!   `getsel.unknown_property`), never an empty match.
//! * `VALUE` is the trimmed remainder (spaces allowed) and must be non-empty.
//! * `TYPE` matches an entity's stored `object.type_key` case-insensitively and
//!   also the friendly geometry-kind alias of its [`cad_domain::SemanticGeometry`]
//!   (`LINE`, `CIRCLE`, `ARC`, `POLYLINE`, `ELLIPSE`, `SPLINE`, `POINT`,
//!   `TEXT`, `MTEXT`, `INSERT`, `DIMENSION`, `HATCH`, `MULTILEADER`, `IMAGE`,
//!   `SHAPE`, `MESH`, `WIPEOUT`/`MASK`, `OPAQUE`, `COMPOUND`).
//! * `LAYER` matches the entity's stored layer **name** (model-space entities
//!   have no instance path, so the stored layer is the effective layer).
//! * `COLOR` / `LINETYPE` match the canonical string projection of
//!   [`crate::property_value`] (`ByLayer`, `ByBlock`, `aci:N`, `#RRGGBB` /
//!   the drawing's linetype name), case-insensitively.
//!
//! The candidate set is **every model-space entity** in the drawing (layer
//! off/frozen state does not filter a QSELECT result; a filter is a data query,
//! not a screen query). Zero matches is **not an error**: `QSELECT` replaces the
//! selection with the (possibly empty) match and reports `qselect.no_match`;
//! `FILTER` adds nothing and reports `filter.no_match`; `GETSEL` behaves like
//! `QSELECT`. `FILTER` on an empty current selection is valid (ADD to empty).
//!
//! `GETSEL` accepts only `LAYER` and `TYPE` (the AutoCAD form); `COLOR` and
//! `LINETYPE` are named refusals there.
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
