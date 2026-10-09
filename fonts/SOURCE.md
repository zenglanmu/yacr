# Provenance of committed fonts

## `osifont.ttf`

- **Upstream project**: osifont — <https://github.com/hikikomori82/osifont>
  (ISO 3098 CAD technical drawing font, created from scratch with free tools).
- **Copy committed here**: QCAD's bundled build,
  <https://github.com/qcad/qcad> `fonts/osifont.ttf`.
- **Obtained from**: `https://cdn.jsdelivr.net/gh/qcad/qcad@master/fonts/osifont.ttf`
  on 2026-10-09 (jsDelivr mirror of the `qcad/qcad` `master` branch).
- **Size / SHA-256**:
  - committed file: `116512` bytes,
    `247ba237597538574a72e2ffe0c03087f1a35099bfff15a79a5fbe3549a258b9`.
  - upstream `master` (`osifont.ttf`) at the same date: `126552` bytes,
    `31e457a464b27ad0e3137bf957f0f4044ed9a3678df91eea2aea55c97c677208`
    (recorded for reference; not committed).
- **Licence**: GNU GPL version 3 **with the GPL font exception** (the font may
  be embedded in documents without those documents becoming GPL). The upstream
  `README.md` states the font is offered under GPLv3/GPLv2/LGPLv3, each with the
  font exception; `osifont.ttf` is the GPLv3+exception build. The GPLv3 text is
  shipped as `COPYING.GPL-3`. This project is AGPLv3, which is compatible with a
  GPLv3 component.
- **Why it is here**: it is the default outline fallback face for CAD text whose
  own font is missing, so bundled desktop/web output can shape text without a
  network fetch. See `docs/fonts.md`.

## Not committed

QCAD's `.cxf` (CAM Expert Font) files are not committed or supported: the
shaping engine handles SHX and sfnt/WOFF only. `mlightcad/cad-data` fonts are
not committed either — they are fetched, never vendored, and their upstream
redistribution terms are the deployer's responsibility.
