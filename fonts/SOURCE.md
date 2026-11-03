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

## `NotoSansCJKsc-Regular.otf`

- **Upstream project**: Noto Sans CJK (Simplified Chinese) —
  <https://github.com/notofonts/noto-cjk>.
- **Pinned source**: commit `f8d157532fbfaeda587e826d4cd5b21a49186f7c`, path
  `Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf`.
  Raw URL:
  <https://raw.githubusercontent.com/notofonts/noto-cjk/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf>
- **Size / SHA-256**:
  - committed file: `16437364` bytes,
    `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`.
- **Licence**: SIL Open Font License 1.1 (OFL-1.1), shipped as
  `NotoSansCJK-OFL.txt`. The file is redistributed **unmodified**; the OFL
  Reserved Font Name clause therefore requires no rename (the RFN is `Source`,
  inherited from Adobe's Source Han Sans; this repository does not modify the
  font).
- **Why it is here**: it is the single bundled fallback face for CJK text whose
  own drawing font is missing, on every host (Linux, Windows, macOS, Android,
  Web). Without it a CJK label would be reported as an unresolved font instead of
  being shaped. See `docs/fonts.md`.

## Not committed

QCAD's `.cxf` (CAM Expert Font) files are not committed or supported: the
shaping engine handles SHX and sfnt/WOFF only. `mlightcad/cad-data` fonts are
not committed either — they are fetched, never vendored, and their upstream
redistribution terms are the deployer's responsibility.
