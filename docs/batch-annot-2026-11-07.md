# 2026-11-07 annotation/group batch: GROUP / UNGROUP / GROUPEDIT / QLEADER / LEADER / TEXTFIT / HATCHSETBOUNDARY / HATCHSETORIGIN / XDLIST (partial)

Status: **contract-level implementation only**. No GUI run, no real DWG, no GPU
and no frontend test run were performed for this batch. Everything below is
synthetic in-process evidence unless explicitly marked otherwise. Windows,
macOS, Android and Web were **NOT VERIFIED**; only the Linux workspace contract
tests were run.

This batch implements ten AutoCAD-2026 style 2D annotation/group names by
reusing existing document authority and geometry paths. No second `cad-db`
document, no second geometry authority and no acadrust patch were introduced.

| # | Command | Status | Boundary |
|---|---|---|---|
| 1 | `GROUP` | implemented (session-only) | `CREATE <name>` / `LIST` / `DELETE <name>`; bare `GROUP` lists. Bounded to 64 case-insensitively unique names. **No DWG group object is created.** |
| 2 | `-GROUP` | alias | normalizes onto `GROUP` (`COMMAND_ALIASES`), same `CommandId`/path |
| 3 | `UNGROUP` | implemented (session-only) | `UNGROUP <name>` removes the name only; every entity is retained |
| 4 | `GROUPEDIT` | implemented (session-only) | `<name> ADD|REMOVE|RENAME <arg>`; membership from the current selection or typed entity ids |
| 5 | `QLEADER` | implemented via MULTILEADER | `<arrow x,y> <landing x,y> <text>`; the legacy QLEADER entity maps to `MULTILEADER` |
| 6 | `LEADER` | implemented via MULTILEADER | same input/path as `QLEADER`; the legacy LEADER entity maps to `MULTILEADER` |
| 7 | `TEXTFIT` | implemented (estimated width only) | `<target width>` changes only the glyph `height` under `height × char count`; true width stretch is **NOT IMPLEMENTED** |
| 8 | `HATCHSETBOUNDARY` | implemented (closed loop subset) | exactly one hatch + closed `Polyline`/whole `Circle`/full `Ellipse`; one transaction/undo; rebuild failure is a named refusal |
| 9 | `HATCHSETORIGIN` | named refusal | the hatch domain stores **no pattern-origin field**, so `HATCHSETORIGIN <x> <y>` changes nothing |
| 10 | `XDLIST` | read-only report (not retained) | this build retains no xdata; every selected entity reports `xdlist.not_retained`, nothing is invented |

## Wiring

Each command has a `CommandId`, a `COMMAND_TABLE` row (except `-GROUP`, which is
an alias), an `Application::execute` arm, work-mode classification where it
writes, a UI catalog row, gating (argument spec/selection minimum/document
scope), and both i18n locales.

* `CommandId`: `Group`, `Ungroup`, `GroupEdit`, `QLeader`, `Leader`, `TextFit`,
  `HatchSetBoundary`, `HatchSetOrigin`, `XdList`.
* `COMMAND_TABLE`: `GROUP`, `UNGROUP`, `GROUPEDIT`, `QLEADER`, `LEADER`,
  `TEXTFIT`, `HATCHSETBOUNDARY`, `HATCHSETORIGIN`, `XDLIST`.
* `COMMAND_ALIASES`: `-GROUP` → `GROUP`.
* Typed arguments: `cad-ui-core::session::annot_arguments` with selectors in
  `ARGUMENT_SELECTORS`; dispatches `CommandPayload::Annotation`.
* Application: `cad-app::app_drawing::annot_batch` (group store:
  `cad-app::groups`).
* Work-only (writes or declares a write): `QLeader`, `Leader`, `TextFit`,
  `HatchSetBoundary`, `HatchSetOrigin`. `Group`/`Ungroup`/`GroupEdit` are
  session-only and `XdList` is read-only, so Viewer mode can reach them.
* Docs: long-form contract in `crates/cad-app/src/command_id/annot_batch.rs`.

## Per-command implementation and exact boundaries

### GROUP / -GROUP

Storage is `cad_app::EntityGroups`: a session-only name → ordered
`SelectionRef` list. It is **not** a DWG object. There is no group table in
`cad-db` and no group geometry in `cad-domain`, so:

* no database write, revision bump, `ChangeSet` or `cad-history` entry;
* nothing survives a session rebuild / document replacement;
* an imported DWG containing `GROUP` objects contributes only its ordinary
  entities (there is no importer path into this store);
* no entity's layer, visibility, geometry or selection changes.

Typed forms:

* bare `GROUP` and `GROUP LIST` — read-only report of every stored name and its
  member count, plus `group.empty` when the store is empty.
* `GROUP CREATE <name>` — captures the **current selection** (refs are
  validated against the open drawing). Empty selection is
  `group.selection_required`; a case-insensitive duplicate is `group.duplicate`;
  a full 64-name store is `group.cap` (explicit, never a silent drop).
* `GROUP DELETE <name>` — removes the name; every entity is retained. Unknown
  names are `group.unknown`.

Names are one non-whitespace token in this build (the grammar deliberately
rejects `CREATE two words`, because `GROUPEDIT <name> ADD` would be ambiguous).
`-GROUP` is an alias, not a second `CommandId`.

### UNGROUP

`UNGROUP <name>` removes that one group from the session store and keeps every
entity. Refusals: `ungroup.needs_name` (bare), `ungroup.payload`, and
`ungroup.unknown`. No transaction, no history, no entity write.

### GROUPEDIT

`GROUPEDIT <name> ADD <arg>` / `REMOVE <arg>` / `RENAME <arg>`.

* `ADD`/`REMOVE` use the current selection when it is non-empty; otherwise
  `<arg>` is comma/space separated model-space entity ids. Unknown ids are
  `groupedit.stale_ref`; duplicate ids and id `0` are
  `groupedit.invalid_entity`; no selection and no ids is
  `groupedit.selection_required`.
* `RENAME` changes only the stored name, validates case-insensitive uniqueness,
  and allows a case-only rename (the display casing follows the request).
* A run that changes nothing reports `groupedit.unchanged`; a real run reports
  `groupedit.added` / `groupedit.removed` / `groupedit.renamed`.

One session mutation per run; **no entity/database write**, no revision bump and
no undo entry.

### QLEADER / LEADER

Both names create **one `MULTILEADER`** through the existing
`Application::create_mleader` path. Style resolution (`Standard` MLEADERSTYLE),
content checks, text length/emptiness, geometry policy and the one-transaction /
one-undo funnel are the same code — no second construction path and no
`AcDbLeader` entity. The typed input is
`QLEADER|LEADER <arrow x,y> <landing x,y> <text>`; points are on the `+Z` work
plane (`z = 0`) and the text is the trimmed remainder and may contain spaces.
The legacy LEADER/QLEADER entity is therefore mapped to MULTILEADER. Refusals
are `qleader.*` / `leader.*` plus every refusal the MLEADER path already names.

### TEXTFIT

`TEXTFIT <target width>` acts on the current selection of directly addressable
model-space `Text`/`MText`. Because the domain has **no width-factor field**, the
command changes only the glyph `height` so the batch's one declared estimate
fits:

```
estimated width = height × text.chars().count()
new height      = target_width / text.chars().count()
```

The estimate is axis-aligned and ignores rotation and real glyph metrics.
AutoCAD's true width stretch (DXF group 41) is **NOT IMPLEMENTED**. Refusals:
`textfit.needs_width`, `textfit.payload`, `textfit.bad_width` (non-finite or
`<= 0`), `textfit.selection`, `textfit.not_addressable`, `textfit.target_missing`,
`textfit.not_text`, `textfit.empty_text` and `textfit.degenerate`. A real run
updates every changed entity in **one transaction / one undo step**;
all-unchanged is `textfit.unchanged` with no transaction.

### HATCHSETBOUNDARY

Payload-free; the selection is the input. Requires exactly one selected
`Hatch` plus at least one selected closed boundary:

* closed `Polyline` with at least three finite vertices (bulges preserved);
* whole `Circle` (the same four quarter-arc loop HATCH writes);
* full `Ellipse` (`|sweep| = 2π`, sampled with the boundary builder's 64
  segments).

Every boundary must be coplanar with the hatch's own `HatchPlane`; a projection
round-trip beyond `1e-7 × coordinate scale` is `hatchsetboundary.non_planar`
(the geometry is never silently flattened). The hatch is rebuilt through the
single domain constructor `HatchGeometry::new` from the new loops **and** the
existing plane, style, fill, pixel size and pattern type; the old internal seeds
are dropped (they described the replaced boundary). The fill display
is derived, never hand-edited. The old boundary is never kept silently: either
the rebuild succeeds and replaces the loops, or the command refuses with
`hatchsetboundary.rebuild_failed`. Source boundary entities are retained and no
associativity is recorded. One transaction / one undo step.

Refusals: `hatchsetboundary.payload`, `.selection`, `.multiple_hatches`,
`.not_addressable`, `.not_closed`, `.open_ellipse`, `.non_planar`, `.degenerate`,
`.unsupported_boundary` and `.rebuild_failed`. An identical loop set is
`hatchsetboundary.unchanged` with no transaction.

### HATCHSETORIGIN

`HATCHSETORIGIN <x> <y>` is parsed and modelled so the real capability gap can be
named, but it is an **unconditional named refusal**:
`cad_domain::HatchGeometry` stores `HatchPlane.normal`/`elevation` and per-line
`HatchPatternLine.base_x`/`base_y`, but **no hatch pattern-origin field**
corresponding to the DXF group-10 origin. Writing those other fields would not
be the requested origin. The refusal is
`hatchsetorigin.missing_field: ... HatchGeometry has no hatch pattern-origin field ...`;
nothing is read from the selection for a write, no transaction opens and no
history entry is recorded. A bare selector is
`hatchsetorigin.needs_argument`; non-finite values are
`hatchsetorigin.bad_point`.

### XDLIST

`XDLIST` is a read-only report of the selected entities' preserved extended data.
`cad-import-acadrust` can summarise source `ExtendedEntityData`/`object_xdata`
during import, but `cad-domain::SemanticGeometry` and `cad-db::DbEntity` **do not
retain xdata**. The command therefore emits one `xdlist.not_retained` diagnostic
per selected entity plus `xdlist.summary`, never a fabricated application name or
value. Refusals: `xdlist.payload`, `xdlist.selection` and
`xdlist.not_addressable`. No write, revision bump or undo entry; Viewer-legal.

## Tests

New contract tests:

* `crates/cad-app/src/tests/drawing/annot_batch.rs` —
  `GROUP` create/list/delete/duplicate/unknown/empty-selection; `UNGROUP`;
  `GROUPEDIT` add/remove/rename/unknown/stale id; `QLEADER` and `LEADER` create
  one MULTILEADER and undo; `TEXTFIT` height fit/undo + refusals;
  `HATCHSETBOUNDARY` loop rebuild/undo + open/non-planar/unsupported refusals;
  `HATCHSETORIGIN` missing-field refusal; `XDLIST` retained/not-retained.
* `crates/cad-ui-core/src/session/tests/tests_commands/annot_arguments.rs` —
  selector admission, `-GROUP` alias, normalization, work-mode classification
  and every typed grammar.
* Exhaustive vocabulary fixtures updated:
  `command_vocabulary.rs`, `snap_grid.rs`, frontend `test/helpers.ts`,
  `view3d.test.ts`, `zoom-argument.test.ts` and `point-display-sysvar.test.ts`.

All assertions are synthetic/in-process. No real DWG, GUI, GPU, Android, iOS,
Windows or macOS evidence is claimed.

## Verification actually run

Run in this worktree with
`CARGO_TARGET_DIR=/tmp/opencode/target-annot-batch CARGO_INCREMENTAL=0
CARGO_PROFILE_DEV_DEBUG=0`:

* `cargo test -p cad-app --locked` — **PASS** (1902 passed, 3 ignored; the
  ignored tests are the pre-existing fixture-gated ones).
* `cargo test -p cad-ui-core --locked` — **PASS** (454 passed, 1 ignored).
* `cargo check -p cad-app -p cad-ui-core --locked` — **PASS**.
* `cargo fmt --all -- --check` — **PASS**.
* `cargo clippy -p cad-app -p cad-ui-core --all-targets --locked -- -D warnings`
  — **PASS**.
* `python3 scripts/check-file-size.py` — **PASS**.
* `python3 scripts/check-i18n.py` — **PASS**.
* `python3 scripts/check-command-docs.py` — **PASS**.
* `python3 scripts/check-architecture.py` — **PASS**.
* Frontend `npm test` / `npm run build` — **NOT RUN**: this checkout has no
  `apps/desktop-ui/node_modules`. The catalog/gating/test-expectation edits are
  therefore compile-unverified on the JS side in this environment.

Not run: GUI hosts, real DWG fixtures, GPU/pixel checks, packaging and every
non-Linux carrier. Those are **NOT VERIFIED**, not passed.
