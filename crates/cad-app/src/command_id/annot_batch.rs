//! Long-form contracts for the 2026-11-07 annotation/group batch:
//! `GROUP` / `-GROUP`, `UNGROUP`, `GROUPEDIT`, `QLEADER`, `LEADER`, `TEXTFIT`,
//! `HATCHSETBOUNDARY`, `HATCHSETORIGIN` and `XDLIST`.
//!
//! The implementation lives in [`crate::app_drawing::annot_batch`]. This module
//! is the contract and names every boundary, so no command silently does less
//! than it says.
//!
//! # `GROUP` / `-GROUP`
//!
//! ## Storage
//!
//! A group is **document data**, not a session value: [`cad_db::DrawingDatabase::groups`]
//! holds `tables/groups.json` rows (a table-local sequence, name, description,
//! DXF selectable flag and ordered member entity ids). It is written to
//! `.ymodel` (format minor 17), imported from DXF/DWG `AcDbGroup` objects and
//! exported back to them. There is no second session authority: `SESSION`'s
//! `textmask_groups` is TEXTMASK's private text/mask association and is never
//! read by these commands.
//!
//! Each create/delete/rename/member edit is **one transaction, one revision,
//! one `ChangeSet` and one undo step** (`ChangeScope::tables = {Groups}`); undo
//! and redo restore the exact row through the same validated write path. A
//! read-only report or a named refusal bumps no revision and records no history.
//!
//! ## Typed forms
//!
//! | line | behaviour |
//! | --- | --- |
//! | `GROUP` (bare) | report every stored name, read-only |
//! | `GROUP LIST` | the same report |
//! | `GROUP CREATE <name>` | capture the current selection under `<name>` |
//! | `GROUP DELETE <name>` | remove the named group; every entity is retained |
//!
//! `-GROUP` is an alias in
//! [`cad_ui_core::session::COMMAND_ALIASES`](https://docs.rs/cad-ui-core), not a
//! second `CommandId`: it normalises onto `GROUP` and therefore the same path.
//!
//! Names are stored trimmed and unique **case-insensitively**. An unquoted name
//! is one token; a name with spaces (legal in a source drawing and in
//! `tables/groups.json`) is addressed by double-quoting it. The
//! command-layer cap is [`crate::ENTITY_GROUP_CAP`] = 64 groups; creating one
//! more is the named `group.cap` refusal rather than a silent drop. A duplicate
//! name is `group.duplicate`; an empty selection is `group.selection_required`;
//! a ref that no longer resolves, names a block-instance member or repeats an
//! entity is `group.stale_ref`; an unknown delete name is `group.unknown`. All
//! refusals happen before the transaction and nothing is written.
//!
//! `GROUP` is Work-only: it writes the drawing's group table.
//!
//! # `UNGROUP`
//!
//! `UNGROUP <name>` removes exactly one named group and **keeps every entity**.
//! The entities are not deleted, moved, unselected or rewritten; only the
//! `tables/groups.json` row is removed (one revision, one undo step). Refusals:
//! `ungroup.needs_name` (a bare selector), `ungroup.unknown` (the name does not
//! resolve), `ungroup.payload` (a non-annotation payload). Work-only.
//!
//! # `GROUPEDIT`
//!
//! `GROUPEDIT <name> ADD <arg>` / `REMOVE <arg>` / `RENAME <arg>`.
//!
//! * `ADD` and `REMOVE` use the **current selection** when it is non-empty;
//!   when the selection is empty they parse `<arg>` as comma-separated
//!   `EntityId` values in the command's document. Both forms are validated
//!   before the transaction: a duplicate entity id, a non-existent entity, a
//!   block-instance/sub-element ref, or an empty input list is refused.
//! * `RENAME` changes only the group name, with the same trimmed,
//!   case-insensitively unique rule as `GROUP CREATE`. Membership is unchanged.
//!
//! One revision and one undo step per real change; a run that adds/removes/
//! renames nothing reports the named `groupedit.unchanged` outcome with no
//! write. Refusals: `groupedit.needs_argument`, `groupedit.payload`,
//! `groupedit.unknown`, `groupedit.rename_duplicate`, `groupedit.stale_ref`,
//! `groupedit.invalid_entity` (a typed id is not a positive entity id) and
//! `groupedit.selection_required` (no selection and no typed ids). Work-only.
//!
//! # `QLEADER` / `LEADER`
//!
//! Both names create **one `MULTILEADER`** through the existing
//! [`crate::CommandId::MLeader`] creation path
//! ([`crate::app_drawing::mleader`]): the same `Standard` `MLEADERSTYLE`
//! resolution, geometry policy, content checks and one-transaction / one-undo
//! funnel. The legacy AutoCAD `LEADER`/`QLEADER` entity is therefore **mapped to
//! `MULTILEADER`**; no `AcDbLeader` entity is produced by this build.
//!
//! Payload is [`crate::AnnotationBatchPayload::Leader`] `{ arrow, landing, text }`.
//! The typed grammar is
//! `QLEADER <arrow x,y> <landing x,y> <text>` (or the `LEADER` spelling); the
//! text is the whole trimmed remainder and must satisfy the shared `TEXT` rules
//! (at most [`crate::MAX_TOOL_TEXT_CHARS`] characters). An empty remainder
//! creates a **content-less** leader (arrow, leader line and dogleg only).
//! Points are flattened to the `+Z` work plane (`z = 0`), exactly like the other
//! typed point forms.
//!
//! Refusals happen before the transaction opens: `qleader.payload` /
//! `leader.payload`, the two coordinate-pair grammars (an unparsable pair is
//! refused by the session before IPC), and every refusal the MLEADER creation
//! path already names (`tool.text_too_long`, `mleader.style_missing`, a non-finite
//! point, a landing point too close to the dogleg, a missing active layer). One
//! entity / one transaction / one undo step. Work-only.
//!
//! `QLEADER`/`LEADER` always create the **text** content kind (MText, or a
//! content-less leader for an empty text) because their payload has no `CONTENT`
//! field; the block and tolerance kinds are reached through `MLEADER CONTENT …`
//! ([`crate::app_drawing::mleader`]), which writes the same `MULTILEADER` entity
//! (entity `172` `1`/`2`/`3`, style `170`).
//!
//! # `TEXTFIT`
//!
//! `TEXTFIT <target width>` acts on the current selection of directly
//! addressable model-space `Text`/`MText` entities. It changes **only** the
//! stored horizontal width factor (DXF TEXT group 41) and uses the one declared
//! estimate this batch shares with the 2D text batch:
//!
//! ```text
//! estimated unstretched width = height × text.chars().count()
//! new width factor            = target_width / estimated unstretched width
//! ```
//!
//! Height, position, justification, rotation, content and style are copied
//! unchanged; only the factor moves, so the rendered advance is stretched. The
//! result is exact **for the estimate**, not for real glyph metrics (the box is
//! axis-aligned and ignores rotation): every written entity carries a
//! `textfit.estimated_width` diagnostic naming the estimate and the factor,
//! plus the batch-level `textfit.applied`. Text with an empty string has no
//! width estimate and is the named `textfit.empty_text` refusal rather than an
//! invented factor.
//!
//! Refusals (nothing written, no transaction): `textfit.needs_width` (a bare
//! selector), `textfit.payload`, a non-finite or non-positive target
//! (`textfit.bad_width`), `textfit.selection` (empty), `textfit.not_addressable`
//! (another document, a block member or a sub-element), `textfit.target_missing`,
//! `textfit.not_text` (a whole-command refusal, as in `SCALETEXT`) and
//! `textfit.empty_text`. A selection that already stores the fitted factor is
//! `textfit.unchanged` with no transaction. A real run updates every changed
//! entity in **one transaction / one undo step**. Work-only.
//!
//! # `HATCHSETBOUNDARY`
//!
//! `HATCHSETBOUNDARY` (payload-free; the selection is the input) requires
//! **exactly one** selected `Hatch` plus at least one selected closed boundary
//! entity. The accepted boundary shapes are the same closed set `BOUNDARY`
//! discovers:
//!
//! * a closed `Polyline` with at least three finite vertices (its bulges are
//!   preserved);
//! * a whole `Circle` (stored as the same four quarter-arc loop HATCH writes);
//! * a full `Ellipse` (`|sweep| = 2π`, sampled with the same 64 segments the
//!   boundary builder uses).
//!
//! Every boundary must be coplanar with the hatch's own `HatchPlane` (a vertex
//! projected onto the plane must round-trip within `1e-7` of its coordinate
//! scale); a non-planar, open or partial boundary is refused rather than
//! flattened. The hatch is rebuilt through the single domain constructor
//! [`cad_domain::HatchGeometry::new`] from the new loops and the **existing**
//! plane, style, fill, pixel size and pattern type; the stored internal seeds
//! described the old boundary, so they are dropped instead of being left
//! pointing at replaced loops. The old boundary is never kept silently: either
//! the rebuild succeeds and replaces the loops, or the command refuses.
//!
//! One `update_entity` in **one transaction / one undo step**; the boundary
//! source entities are retained and no associativity is recorded. Refusals:
//! `hatchsetboundary.payload`, `hatchsetboundary.selection` (no hatch, or no
//! boundary entities), `hatchsetboundary.multiple_hatches`,
//! `hatchsetboundary.not_addressable`, `hatchsetboundary.not_closed`,
//! `hatchsetboundary.open_ellipse`, `hatchsetboundary.non_planar`,
//! `hatchsetboundary.degenerate` and `hatchsetboundary.rebuild_failed` (the
//! domain constructor rejects the loop set, e.g. a self-intersection). A loop set
//! equal to the stored one is `hatchsetboundary.unchanged` with no transaction.
//! Work-only.
//!
//! # `HATCHSETORIGIN`
//!
//! `HATCHSETORIGIN <x> <y>` places the pattern origin of **exactly one**
//! directly addressable model-space hatch on the given hatch-plane point, in one
//! transaction / one undo step. The stored field is the same one
//! [`cad_domain::HatchGeometry::pattern_origin`] exposes — the phase
//! displacement applied to the pattern's definition rows — because neither the
//! DXF nor the DWG hatch record has a dedicated origin group (the phase lives in
//! each row's base point, groups 43/44). The argument, however, is the **absolute**
//! origin: [`cad_domain::HatchGeometry::absolute_pattern_origin`] is the composed
//! base point of the first definition row (the hatch-level angle and scale
//! applied, then the displacement added), and the written displacement is
//! `d + (P − anchor)`, i.e. `P` minus that composed base. For a hatch whose first
//! row has base `[0, 0]` and no displacement the two editors agree; for an
//! imported hatch whose phase lives in the rows (`base != [0, 0]`) they
//! deliberately differ, and the rows are never ignored. All other stored fields
//! are preserved and the display is rebuilt through the domain constructor.
//!
//! A bare selector is `hatchsetorigin.needs_argument`; any other payload is
//! `hatchsetorigin.payload`; non-finite coordinates are
//! `hatchsetorigin.bad_point`. `hatchsetorigin.selection` (not exactly one ref),
//! `hatchsetorigin.not_addressable`, `hatchsetorigin.not_hatch` and
//! `hatchsetorigin.not_pattern` (a solid/gradient fill draws no rows) refuse
//! before any write. `hatchsetorigin.missing_field` is the narrowed capability
//! refusal: the pattern stores **no definition rows** (a user-defined pattern
//! carrying only a spacing), so this model cannot name an origin to place; and
//! `hatchsetorigin.unrepresentable` refuses a target displacement that is not
//! finite. Repeating the current origin is `hatchsetorigin.unchanged` with no
//! transaction. Work-only.
//!
//! # `XDLIST`
//!
//! `XDLIST` (payload-free; the selection is the input) is a **read-only** report
//! of the selected entities' preserved extended data. `cad-connector-acadrust` can
//! summarise source `ExtendedEntityData`/`object_xdata` during import, but
//! `cad-domain::SemanticGeometry` and `cad-db::DbEntity` **do not retain xdata**:
//! there is no application name, no values and no per-entity store to read back.
//!
//! The command therefore reports `xdlist.not_retained` once and never invents a
//! record. If a future domain/db layer retains xdata, this command gains the
//! real list; until then the named report is the complete answer. Refusals:
//! `xdlist.payload` (a supplied payload), `xdlist.selection` (empty) and
//! `xdlist.not_addressable` (another document, a block member or a sub-element).
//! There is no write, revision bump or undo entry, so Viewer mode allows it.
