//! Long-form contracts for the 2026-11-07 annotation/group batch:
//! `GROUP` / `-GROUP`, `UNGROUP`, `GROUPEDIT`, `QLEADER`, `LEADER`, `TEXTFIT`,
//! `HATCHSETBOUNDARY`, `HATCHSETORIGIN` and `XDLIST`.
//!
//! The implementation lives in [`crate::app_drawing::annot_batch`] (except the
//! session group store, [`crate::groups`]). This module is the contract and
//! names every boundary, so no command silently does less than it says.
//!
//! # `GROUP` / `-GROUP`
//!
//! ## Storage boundary
//!
//! A group is a **session-only** value: a name plus an ordered list of
//! [`cad_domain::SelectionRef`]s, held in [`crate::EntityGroups`]. This build has
//! **no DWG group object** (`cad-db` has no group table and `cad-domain` has no
//! group geometry), so:
//!
//! * no database write, revision bump, `ChangeSet` or `cad-history` entry;
//! * nothing survives a session rebuild or a document replacement;
//! * a saved DWG that contains `GROUP` objects is imported as ordinary entities
//!   and does **not** populate this store (there is no importer path for groups);
//! * `GROUP` never changes an entity's layer, selection, visibility or geometry.
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
//! Names are one non-whitespace token, stored trimmed and unique
//! **case-insensitively**. The store is capped at
//! [`crate::ENTITY_GROUP_CAP`] = 64 groups; creating one more is the named
//! `group.cap` refusal rather than a silent drop. A duplicate name is
//! `group.duplicate`; an empty selection is `group.selection_required`; a ref
//! that no longer resolves is `group.stale_ref`; an unknown delete name is
//! `group.unknown`. All refusals happen before the store is mutated and nothing
//! is written.
//!
//! `GROUP` is Viewer-legal: it can never write the drawing.
//!
//! # `UNGROUP`
//!
//! `UNGROUP <name>` removes exactly one named group and **keeps every entity**.
//! The entities are not deleted, moved, unselected or rewritten; only the
//! session's name → refs mapping is dropped. Refusals: `ungroup.needs_name` (a
//! bare selector), `ungroup.unknown` (the name does not resolve),
//! `ungroup.payload` (a non-annotation payload). Session mutation only, so
//! Viewer-legal.
//!
//! # `GROUPEDIT`
//!
//! `GROUPEDIT <name> ADD <arg>` / `REMOVE <arg>` / `RENAME <arg>`.
//!
//! * `ADD` and `REMOVE` use the **current selection** when it is non-empty;
//!   when the selection is empty they parse `<arg>` as comma-separated
//!   `EntityId` values in the command's document. Both forms are validated
//!   before the store is touched: a duplicate entity id, a non-existent entity,
//!   a block-instance/sub-element ref, or an empty input list is refused.
//! * `RENAME` changes only the group name, with the same trimmed,
//!   case-insensitively unique rule as `GROUP CREATE`. Membership is unchanged.
//!
//! One session mutation per run; **no entity/database write**, no revision bump
//! and no undo entry. A run that adds/removes/renames nothing reports the named
//! `groupedit.unchanged` outcome with no mutation. Refusals:
//! `groupedit.needs_argument`, `groupedit.payload`, `groupedit.unknown`,
//! `groupedit.rename_duplicate`, `groupedit.stale_ref`, `groupedit.invalid_entity`
//! (a typed id is not a positive entity id) and `groupedit.selection_required`
//! (no selection and no typed ids). Viewer-legal.
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
//! (non-empty/ non-whitespace and at most
//! [`crate::MAX_TOOL_TEXT_CHARS`] characters). Points are flattened to the `+Z`
//! work plane (`z = 0`), exactly like the other typed point forms.
//!
//! Refusals happen before the transaction opens: `qleader.payload` /
//! `leader.payload`, the two coordinate-pair grammars (an unparsable pair is
//! refused by the session before IPC), and every refusal the MLEADER creation
//! path already names (`tool.text_empty`, `tool.text_too_long`,
//! `mleader.style_missing`, `mleader.text_required`, a non-finite point, a
//! landing point too close to the dogleg, a missing active layer). One entity /
//! one transaction / one undo step. Work-only.
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
//! `HATCHSETORIGIN <x> <y>` is modelled and parsed so the exact capability gap
//! can be named, but it is an **unconditional named refusal**:
//! [`cad_domain::HatchGeometry`] stores the plane (`HatchPlane.normal` +
//! `elevation`) and the per-pattern-line base points
//! (`HatchPatternLine.base_x` / `base_y`), but it has **no hatch pattern-origin
//! field** corresponding to the DXF group-10 origin. Writing any of those fields
//! would not be the requested origin, so the command refuses with
//! `hatchsetorigin.missing_field: HatchGeometry has no pattern-origin field`.
//!
//! A bare selector is `hatchsetorigin.needs_argument`; any other payload is
//! `hatchsetorigin.payload`. The command reads the selection only to report
//! whether the one selected hatch exists; it changes nothing, opens no
//! transaction and records no history. Work-only (its declared intent is a write).
//!
//! # `XDLIST`
//!
//! `XDLIST` (payload-free; the selection is the input) is a **read-only** report
//! of the selected entities' preserved extended data. `cad-import-acadrust` can
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
