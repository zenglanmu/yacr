# Bundled CAD drawing fonts (`fonts/`)

This directory is the repository's **committed CAD-text font package**. It is
the single source the hosts copy next to their output:

- **Linux desktop / CLI**: the package builder copies this directory next to
  the executable as `fonts/` (sibling of the binary, or of `bin/`); the host
  auto-loads `fonts/fonts.json` at startup. See `docs/fonts.md`.
- **Web**: `scripts/build-web.sh` copies it into `web-dist/fonts/`; the browser
  host serves the same catalogue from there (or from the CDN, see below).

## Contents

- `osifont.ttf` — QCAD's ISO 3098 technical drawing font (GPL-3 with the GPL
  font exception). This is the default outline face used when a drawing's own
  font cannot be resolved. Provenance, source URLs and hashes are in
  `SOURCE.md`.
- `fonts.json` — the catalogue in the `mlightcad/cad-data` JSON format
  (`{ file, name[], type }`) so `cad-resources::FontCatalog` can index it.
- `COPYING.GPL-3` — the licence text shipped with `osifont.ttf`.

## Scope and limits

- Only fonts that may be redistributed under this repository's terms are
  committed here. The `mlightcad/cad-data` catalogue is **not** committed: the
  web host fetches it from its CDN at runtime (or a build-time download copies
  it, without committing it).
- QCAD's compiled `.cxf` fonts are **not** supported by the shaping engine
  (`cad-representation` reads SHX and sfnt/WOFF only); they are recorded as
  unsupported, never silently treated as usable. See `docs/fonts.md`.
- This package is intentionally small. A drawing that references a font it does
  not contain falls back to the default outline face, and if even that is
  missing the text is reported, not dropped.
