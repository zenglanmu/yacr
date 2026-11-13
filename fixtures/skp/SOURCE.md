# SketchUp (`.skp`) fixtures — external only

**There is no committed `.skp` fixture in this repository, and this directory
intentionally contains none.** Every `.skp` the import lane is tested against is
**external** (`YACR_TEST_SKP` / `YACR_TEST_SKP_2026`) and lives outside the
repository.

Why nothing is committed:

- The only real files used so far are the **`skpwright` corpus**. It is not ours
  to redistribute: the models are third-party content, and several carry
  **embedded texture images that are themselves third-party material**. A
  `.skp` is a self-contained archive, so committing one redistributes those
  images too.
- The header of a `.skp` also carries a per-model GUID (a per-model identifier).
  That is not a licence question, but it is more per-model metadata than a
  contract fixture needs.
- There is no synthetic `.skp` writer in this repository: the container is an
  MFC `CArchive` object stream (releases 3–8, 2013–2020) or a ZIP/tagged-record
  archive (2026/2027), and hand-authoring one would create a fixture that only
  proves the reader agrees with our own writer.

Nothing here is a compatibility claim or product-acceptance evidence, and no file
is added to `fixtures/manifest` (there is no committed file to declare).

## Running the opt-in tests

```bash
# A release 3–8 / 2013–2020 (stream container) model, e.g. box.skp
YACR_TEST_SKP=/absolute/path/box.skp \
  cargo test -p cad-import-skp --locked

# A 2026/2027 (ZIP/tagged-record container) model
YACR_TEST_SKP_2026=/absolute/path/theater-2026.skp \
  cargo test -p cad-import-skp --locked
```

- Both variables take an **absolute path**; a relative path is the caller's
  problem, not silently resolved.
- **Unset means skip.** A skip is **not** a run and **not** evidence. A CI run
  without these variables proves only the 19 in-crate contract tests (release
  ceilings, materials, triangulation, visibility pruning, explicit refusals).
- No download script is provided: the corpus is not redistributable, so the
  operator supplies the file.

## What the external corpus established (bounded, not golden)

A bounded scan of the `skpwright` corpus (763 `.skp` files, external, not
committed) produced: **728 imported, 25 refused `import.skp.edge_only_geometry`,
10 refused `import.skp.no_geometry`, 0 failures, 0 panics**. The two theater
models (2017 stream container and 2026 ZIP container) hold the same geometry:
1246 entities each, 96,333 vs 96,344 triangles, surface area **1020.981869 m²**
in both (relative difference 5e-14).

Those numbers are a bounded local experiment. They are **not** a golden
regression, not a performance measurement, and not product acceptance; see
`docs/compatibility.md` and `docs/handoff.md` for the current evidence boundary.
