# ymodel Phase 1 inventory: in-memory state → `.ymodel` entries

Phase 1 acceptance artifact required by ADR-0037 D17. Every field the database and document
hold is listed once, with the archive entry that carries it and the rule that governs the
mapping. It exists so a future field addition cannot quietly fall outside the format.

Evidence status: the tables below were produced by reading the code at `4faef9ef`
(`crates/cad-db/src/drawing/mod.rs`, `crates/cad-domain/src/semantic.rs`,
`crates/cad-app/src/lib.rs`). Counts marked here were verified by counting the declarations in
source (`grep -c '^\s*pub(crate) '`), not by running a test. **No `.ymodel` archive has been
written or read yet** — the store is not implemented. Anything below is a design contract, not
a verified behaviour.

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

## 2. `cad_app::Document` — 5 fields

`crates/cad-app/src/lib.rs:191`. Three move into `cad-domain::ymodel::DocumentHeader`.

| field | type | destination |
|---|---|---|
| `id` | `DocumentId` | stay (host/session identity) |
| `drawing` | `Arc<DrawingDatabase>` | stay (the payload) |
| `identity` | `DocumentIdentity` | `header.identity` — the type already lives in `cad-domain` (`lib.rs:357`); only the codec is new |
| `units` | `UnitContext` | `header.units` |
| `resource_keys` | `Vec<String>` | `header.resource_keys` |

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
  Style 5, DimStyle 6, MLeaderStyle 7, Linetype 8, Scale 9, Model 10, SubCategory 11,
  Relationship 12, Aspect 13.
- `(kind << 45) | seq`, `seq < 2^45`, whole value ≤ 2^53 − 1 (exact in `jq`/`JSON.parse`).
- Model ids are derived, not allocated: model space = 0, paper = layout seq, block = block seq,
  `cad:DefinitionModel` = reserved `MAX_SEQ`.
- **Nine** high-water marks. The code today has eight; linetypes have no allocator and recompute
  `max(row id) + 1` on every create (`crates/cad-db/src/drawing/linetype_edit.rs:258-277`), so
  create → delete → create already reuses an id. Phase 1 must add `next_linetype_id`.

## 5. Dirty scope → entries

`ChangeScope` maps to entries as: `shards`/`prev_shards` → `elements/<model>-<shard>.json`;
`tables` → `tables/*.json`; `header` → `document.json`. `manifest.json` is derived from the
whole document and is always rewritten. Nine revision-bump sites must fill it, plus the
display-state writer and the builder-excluded import path; the ordered list is in
ADR-0037 D12 and in the write-path recon that produced this document.

## 6. Open defect found by the archive acceptance test

**Paper-space and block models collide on their persisted id.** `ModelKey::Paper(LayoutId(n))` and
`ModelKey::Block(BlockId(n))` both encode as `(Model << 45) | n`, so a drawing that has a paper
layout and a block with the same numeric id — which is almost every real drawing, since both
allocators start at 1 — cannot be persisted: `write` fails with a duplicate element entry, or, when
the block happens to be empty, `read` fails with a duplicate model record.

Cause: model ids were derived from the owning layout/block sequence under a single `IdKind::Model`,
so the kind carries no family information and the sequence alone cannot disambiguate. Fix: one
`IdKind` per model family (`ModelSpace`, `PaperModel`, `BlockModel`, `DefinitionModel`), which also
removes the reserved-`MAX_SEQ` trick for the definition model. Not fixed at the time of writing;
`crates/cad-store-zip/tests/document_fixed_point.rs` documents the workaround (block ids 2/3) and
must be simplified to colliding ids once the fix lands.

## 7. Known follow-ups (deliberately not in Phase 1)

1. `next_linetype_id` does not exist yet — Phase 1 must add the field, the mint path, the
   builder seeding, and the migration note.
2. `Revision` is `u64` while ids are `u128`; the store range-checks both, and `DbObject.revision`
   is always `Revision(0)` today (it is not the element's persisted revision).
3. `block_name_order` and the two display side tables are the only fields where "derive it"
   was rejected in favour of persisting; each carries a comment in the payload codec.
4. `manifest.sha256_c` cannot be computed or verified with `zip` 9.5.0 (crate-private raw
   reader), so it is carried through from the previous manifest and marked unverified; every
   entry is verified on its decompressed `sha256` + `len` + CRC instead. Deciding whether to
   drop the field (minor bump) or keep it belongs to Phase 2.
5. `TableKind` has no `ALL` constant, so the store keeps its own `table_entry` mirror; a new row
   table needs both, and a missed one is refused as `UNSUPPORTED_MINOR` rather than dropped.
6. Style / layer / linetype / scale / layout **and block definition** rows are stored as
   `tables/*.json` in v1, not as `DefinitionElement`s (ADR-0037 D7 storage deviation). `ChangeScope::TableKind` is the closed
   set that has to grow with any new row table; a table missing from it would be silently
   unsaved. Migrating these rows to elements needs a minor bump and a migration.
