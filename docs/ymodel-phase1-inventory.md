# ymodel Phase 1 inventory: in-memory state → `.ymodel` entries

Phase 1 acceptance artifact required by ADR-0037 D17. Every field the database and document
hold is listed once, with the archive entry that carries it and the rule that governs the
mapping. It exists so a future field addition cannot quietly fall outside the format.

Evidence status: the tables below were produced by reading the code at `4faef9ef`
(`crates/cad-db/src/drawing/mod.rs`, `crates/cad-domain/src/semantic.rs`,
`crates/cad-app/src/lib.rs`). Counts marked here were verified by counting the declarations in
source (`grep -c '^\s*pub(crate) '`), not by running a test.

**2026-11-19 update:** the store and carrier wiring now exist (Phase 1 + Phase 2 dirty-shard
raw-copy). This file remains the field-mapping contract — every field still maps to the entry
listed here — but the earlier "no archive has been written or read yet" caveat is obsolete.
Evidence for the mapping is the `cad-store-zip` fixed-point/raw-copy tests and the `cad-host`
`ymodel` contract tests; see `docs/validation-ymodel-phase2-2026-11-19.md`. Real DWG/DXF L1
round-trip and L0 source passthrough remain unverified (they are Phase 2/3 items, not covered by
this inventory).

## 1. `DrawingDatabase` — 39 `pub(crate)` fields

`crates/cad-db/src/drawing/mod.rs:94` (`#[derive(Debug, Clone)]`).

Entry keys: `document` = `document.json`; `table:X` = `tables/X.json`;
`elements` = `elements/<model>-<shard>.json`; `derived` = rebuilt on load;
`runtime` = never persisted.

| # | field | type | entry | rule |
|---|---|---|---|---|
| 1 | `id` | `DatabaseId` | `document` | Save-As mints a new value (D3) |
| 2 | `revision` | `Revision` | `document` | **Not** the saved state: `manifest.saved_revision` is authoritative on load |
| 3 | `construction_nonce` | `u64` | `runtime` | Process-local instance discriminator |
| 4 | `entities` | `BTreeMap<EntityId, DbEntity>` | `elements` | One element per row; shard = `seq/2048` |
| 5 | `layers` | `BTreeMap<LayerId, Layer>` | `table:layers` | Merged with field 19 by `id` |
| 6 | `blocks` | `BTreeMap<BlockId, BlockDefinition>` | `table:blocks` | Member ids, dynamic visibility; a table in v1 (D7 storage deviation) |
| 7 | `block_metadata` | `BTreeMap<BlockId, BlockMetadata>` | `table:blocks` | Merged into the block row (name, base point) |
| 8 | `block_name_order` | `Vec<(String, BlockId)>` | `table:blocks` | Persisted as an ordered array, **not** derived: last-duplicate-wins lookup is order-dependent |
| 9 | `block_source_handles` | `BTreeMap<u64, BlockId>` | `table:blocks` | Source DWG handle per block; not recomputable |
| 10 | `hidden_entities` | `BTreeSet<EntityId>` | `elements` | Folded into the element payload as `invisible` |
| 11 | `layouts` | `BTreeMap<LayoutId, Layout>` | `table:layouts` | `LayoutId(0)` is synthetic model space, not a row |
| 12 | `styles` | `BTreeMap<StyleId, Style>` | `table:styles` | |
| 13 | `linetypes` | `BTreeMap<LinetypeId, LineType>` | `table:linetypes` | Needs `next_linetype_id` (see §4) |
| 14 | `linetype_scale` | `f64` | `document` | `$LTSCALE` |
| 15 | `point_display_mode` | `i16` | `document` | `$PDMODE`, raw |
| 16 | `point_display_size` | `f64` | `document` | `$PDSIZE`, raw (non-finite preserved) |
| 17 | `plot_settings` | `BTreeMap<LayoutId, PlotSettingsRecord>` | `table:plot-settings` | |
| 18 | `render_attributes` | `BTreeMap<EntityId, EntityRenderAttributes>` | `elements` | Folded into the element payload |
| 19 | `layer_attributes` | `BTreeMap<LayerId, LayerAttributes>` | `table:layers` | Merged with field 5 by `id` |
| 20 | `scales` | `BTreeMap<ScaleId, Scale>` | `table:scales` | |
| 21 | `insertion_base` | `Option<Point3>` | `document` | `$INSBASE`; `None` is meaningful |
| 22 | `limits_min` | `Option<[f64; 2]>` | `document` | `$LIMMIN` |
| 23 | `limits_max` | `Option<[f64; 2]>` | `document` | `$LIMMAX` |
| 24 | `dim_styles` | `BTreeMap<DimStyleId, DimStyle>` | `table:dimstyles` | |
| 25 | `dim_style_ids_by_name` | `BTreeMap<String, DimStyleId>` | `derived` | Name index |
| 26 | `mleader_styles` | `BTreeMap<MLeaderStyleId, MLeaderStyle>` | `table:mleaderstyles` | |
| 27 | `mleader_style_ids_by_name` | `BTreeMap<String, MLeaderStyleId>` | `derived` | Name index |
| 28 | `active_annotation_scale` | `Option<ActiveAnnotationScale>` | `document` | `CANNOSCALE` |
| 29 | `code_page` | `Option<String>` | `document` | `$DWGCODEPAGE`, raw |
| 30 | `next_entity_id` | `u128` | `document` | High-water mark, restored verbatim (D5) |
| 31 | `next_block_id` | `u128` | `document` | High-water mark |
| 32 | `next_layer_id` | `u128` | `document` | High-water mark |
| 33 | `next_layout_id` | `u128` | `document` | High-water mark |
| 34 | `next_style_id` | `u128` | `document` | High-water mark |
| 35 | `next_dim_style_id` | `u128` | `document` | High-water mark |
| 36 | `next_mleader_style_id` | `u128` | `document` | High-water mark |
| 37 | `next_scale_id` | `u128` | `document` | High-water mark |
| 38 | `external_references` | `BTreeMap<String, ExternalReference>` | `table:xrefs` | Keyed by xref block name; `tables/sources.json` is raw-source provenance, a different entry |
| 39 | `bounds_cache` | `Arc<RwLock<BoundsCache>>` | `runtime` | Revision-tagged memo |

Count check: 39 declared `pub(crate)` fields — 29 data fields, 8 allocators, `external_references`
and `bounds_cache`. Nine counters are needed (§4); the ninth does not exist yet.

## 2. `cad_app::Document` — 6 fields

`crates/cad-app/src/lib.rs:191`. Three move into `cad-domain::ymodel::DocumentHeader`.

| field | type | destination |
|---|---|---|
| `id` | `DocumentId` | stay (host/session identity) |
| `drawing` | `Arc<DrawingDatabase>` | stay (the payload) |
| `identity` | `DocumentIdentity` | `header.identity` — the type already lives in `cad-domain` (`lib.rs:357`); only the codec is new |
| `units` | `UnitContext` | `header.units` |
| `resource_keys` | `Vec<String>` | `header.resource_keys` |
| `persistence` | `PersistenceState` | `manifest.json` (`open_revision`, `saved_revision`, `document_uuid`, foreign entries) + `source_sha256` for the conflict check |

## 3. `SemanticGeometry` — 20 variants

`crates/cad-domain/src/semantic.rs:104`. Class ids are `(schema << 32) | ordinal` with
`schema = 1` (`cad.geometry`) and ordinals 1–20 in declaration order; the payload is
`{"class": "<type_key>", <fields…>}`. Field lists live in `cad-domain/src/ymodel/descriptor.rs`
and are asserted against the codec by a field-completeness test.

| # | variant | type_key | derived data inside the payload |
|---|---|---|---|
| 1 | `Line` | `cad:Line` | — |
| 2 | `Polyline` | `cad:Polyline` | — |
| 3 | `Circle` | `cad:Circle` | — |
| 4 | `Arc` | `cad:Arc` | — |
| 5 | `Ellipse` | `cad:Ellipse` | — |
| 6 | `Spline` | `cad:Spline` | — |
| 7 | `Point` | `cad:Point` | — |
| 8 | `Mesh` | `cad:Mesh` | — |
| 9 | `Body3d` | `cad:Body3d` | `display` cache |
| 10 | `Insert` | `cad:Insert` | — |
| 11 | `Text` | `cad:Text` | — (style is a reference) |
| 12 | `Shape` | `cad:Shape` | — |
| 13 | `Opaque` | `cad:Opaque` | — (raw bytes, hex-encoded) |
| 14 | `Compound` | `cad:Compound` | — |
| 15 | `Dimension` | `cad:Dimension` | `display`, `measurement` |
| 16 | `Hatch` | `cad:Hatch` | `display` |
| 17 | `MultiLeader` | `cad:MultiLeader` | `display` |
| 18 | `Table` | `cad:Table` | `display` |
| 19 | `Image` | `cad:Image` | — (`adjust` is stored input, not a cache) |
| 20 | `Mask` | `cad:Mask` | — |

Phase 1 keeps the embedded `display`/`measurement` fields verbatim (D8): they are persisted,
not regenerated. That is why the four private-field payloads need `pub(crate) from_persisted`
constructors — routing a load through `synthesized()`/`new()` would recompute display and
silently change stored data, and would reject legacy non-finite rows.

## 4. Identity

- Kinds (file-format constants, never renumbered): Entity 1, Layer 2, Block 3, Layout 4,
  Style 5, DimStyle 6, MLeaderStyle 7, Linetype 8, Scale 9, ModelSpace 10, SubCategory 11,
  Relationship 12, Aspect 13, PaperModel 14, BlockModel 15, DefinitionModel 16.
- `(kind << 45) | seq`, `seq < 2^45`, whole value ≤ 2^53 − 1 (exact in `jq`/`JSON.parse`).
- Model ids are derived, not allocated, and each model family has its own kind: model space =
  `ModelSpace`/0, paper = `PaperModel`/layout seq, block = `BlockModel`/block seq,
  `cad:DefinitionModel` = `DefinitionModel`/0. Paper and block sequences may therefore be equal
  without sharing an id.
- **Nine** high-water marks. The code today has eight; linetypes have no allocator and recompute
  `max(row id) + 1` on every create (`crates/cad-db/src/drawing/linetype_edit.rs:258-277`), so
  create → delete → create already reuses an id. Phase 1 must add `next_linetype_id`.

## 5. Dirty scope → entries

`ChangeScope` maps to entries as: `shards`/`prev_shards` → `elements/<model>-<shard>.json`;
`tables` → `tables/*.json`; `header` → `document.json`. `manifest.json` is derived from the
whole document and is always rewritten. Nine revision-bump sites must fill it, plus the
display-state writer and the builder-excluded import path; the ordered list is in
ADR-0037 D12 and in the write-path recon that produced this document.

## 6. Defect found by the archive acceptance test (fixed)

**Paper-space and block models collided on their persisted id.** `ModelKey::Paper(LayoutId(n))` and
`ModelKey::Block(BlockId(n))` both encoded as `(Model << 45) | n`, so a drawing that has a paper
layout and a block with the same numeric id — which is almost every real drawing, since both
allocators start at 1 — could not be persisted: `write` failed with a duplicate element entry, or,
when the block happened to be empty, `read` failed with a duplicate model record.

Cause: model ids were derived from the owning layout/block sequence under a single `IdKind::Model`,
so the kind carried no family information and the sequence alone could not disambiguate.

**Fixed by** giving each model family its own kind, so the id decodes without a side table and the
reserved-`MAX_SEQ` definition-model trick is gone:

| model | kind | value | sequence |
|---|---|---|---|
| `cad:ModelSpace` | `ModelSpace` | 10 | 0 |
| `cad:PaperSpace` | `PaperModel` | 14 | layout id (never 0) |
| `cad:BlockModel` | `BlockModel` | 15 | block id (never 0) |
| `cad:DefinitionModel` | `DefinitionModel` | 16 | 0 |

Kind 10 keeps its value (and now means model space only); 14–16 are new. `tables/models.json` still
records a readable `kind` spelling, but the reader derives the family from the id and refuses a
record whose two spellings disagree (`INVALID_DOCUMENT`). The acceptance fixture now uses layout 1
with block 1, and
`crates/cad-store-zip/tests/document_fixed_point.rs::paper_and_block_models_with_equal_sequences_do_not_collide`
pins the regression directly.

## 7. Decisions taken during implementation

1. **Block definition rows are a table** (`tables/blocks.json`), like the other row families — same
   D7 storage deviation, and it is why `ChangeScope` routes a block-row write to
   `TableKind::Blocks` while block *members* stay elements of `ModelKey::Block`.
2. **`block_name_order` is persisted** as a per-row `registration_index`, because
   last-duplicate-wins name lookup cannot be rebuilt from an id-keyed map.
3. **Element identity is inside the payload** (`CAD_ELEMENT.id`), so an entry is readable on its
   own and a bare sequence in the file fails the encoded-row contract test.
4. **`document_uuid` is minted once at document creation**, never derived: the store's fallback
   hashes the header, which would make an identity follow its content.
5. **The native save is a transitional rule**: `.dwg` source → DWG export, `.ymodel` or no source
   → `.ymodel`, anything else → DXF. A real format chooser is later work.

## 8. Open items

1. ~~`manifest.sha256_c` cannot be computed or verified with `zip` 9.5.0~~ — **closed by
   ADR-0046**: JSON entries are compressed in-process before the zip write, so `sha256_c` is
   written and verified before decompression. The field is retained; every entry is still also
   verified on its decompressed `sha256` + `len`.
2. `TableKind` has no `ALL` constant, so the store keeps its own mirror of `table_entry`; a new row
   table needs both, and a missed one is refused as `UNSUPPORTED_MINOR` rather than dropped.
3. `snapshot/tables.rs` is at ~998 lines behind a `#[rustfmt::skip]` module, so `cargo fmt` cannot
   reach it. It needs a real module split before more rows land there.
4. `L0` source bytes are not written yet (`raw/source-*.bin`), so the passthrough predicate is
   correct but unsatisfiable. L1 fixed points and performance numbers are **NOT RUN**.
5. Archive adapters now exist on Linux, Windows, macOS, Android and Web (`apps/app-*/…/ymodel_archive.rs`).
   Windows/macOS/Android runtime behaviour remains **NOT VERIFIED**; the Web adapter (2026-11-25,
   ADR-0060) encodes into the browser download and installs before the first document, with
   browser-level open/save **NOT RUN**.
6. `Revision` is `u64` while ids are `u128`; the store range-checks both, and `DbObject.revision`
   is always `Revision(0)` today (it is not the element's persisted revision).
