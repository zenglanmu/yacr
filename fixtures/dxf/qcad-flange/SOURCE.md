# QCAD `flange` sample — provenance

Three third-party files used as a **render regression input and reference**. They
are committed because the QCAD project distributes them openly (see
"Licence" below); they are not user/vendor drawings and carry no personal data.

## Origin

| Field | Value |
|---|---|
| Upstream project | QCAD — The Open Source 2D CAD (<https://github.com/qcad/qcad>) |
| Upstream path | `examples/` |
| File page (DXF) | <https://github.com/qcad/qcad/blob/master/examples/flange.dxf> |
| File page (PNG) | <https://github.com/qcad/qcad/blob/master/examples/flange.png> |
| File page (PDF) | <https://github.com/qcad/qcad/blob/master/examples/flange.pdf> |
| Download URL pattern | `https://raw.githubusercontent.com/qcad/qcad/master/examples/<name>` |
| Upstream commit at download | `dcf5754b0a19d8e57eddd467d18bf802ac12c2e2` (`master`, "cleanup", 2026-09-29) |
| Downloaded | 2026-10-03, via the operator-provided HTTP proxy `http://192.168.8.1:10809` |
| Transport | `curl -fsSL` (no range requests, no transformation) |

Blob SHAs reported by the GitHub contents API and the locally recomputed
SHA-256 digests match, so the committed bytes are unmodified upstream bytes.

| File | Bytes | SHA-256 | Upstream git blob |
|---|---|---|---|
| `flange.dxf` | 287223 | `df469e7e91e38c901fced51c9621c8d2b6b6568678803e563b29c9ecf8dec84b` | `10dd59535ba790af97bffe8fc239c33e71e00873` |
| `flange.png` | 95087 | `26d730aa0a758adbe9cab8e2b05a17266a715649442c49452f53f5098e3cea36` | `78091d35d11eff9e5911ff58f42e492aecce0161` |
| `flange.pdf` | 199962 | `b33726c83edf31710268b9b143574d91520770d2fe704ffbb00ced468c6f830b` | `de1f1f267a0decd2dd253efff0e4188135a9e2ee` |

## What each file is

- `flange.dxf` — AutoCAD 2013 (`AC1027`) ASCII DXF, the drawing under test.
  Header extents are `0,0`–`297,210` mm (`$INSUNITS = 4`, millimetres); the file
  contains model-space geometry plus `AcDbDimension` / `AcDbMText` that this
  project currently imports as `Partial` (no display representation yet).
- `flange.png` — 1024×768 RGBA render produced upstream by QCAD. Used only as a
  **human reference** and for a coarse ink-coverage/layout sanity check; it is
  *not* a project-generated golden image and no pixel-fidelity metric is claimed.
- `flange.pdf` — one-page vector PDF (MediaBox `0 0 1191 842`) produced upstream
  by QCAD, kept so the reference can be inspected as vectors rather than pixels.

## Licence

QCAD's `LICENSE.txt` states that the QCAD 3 source code is distributed under
**GPL version 3 with optional exceptions**, and that *icons and documentation*
are distributed under **Creative Commons Attribution 3.0 Unported (CC BY 3.0)**.
The `examples/` files carry no separate per-file licence notice.

This repository is licensed AGPL-3.0; GPLv3 and AGPLv3 are explicitly
compatible (AGPLv3 §13 permits combining with GPLv3), and CC BY 3.0 only
requires attribution, which is given here. If QCAD's upstream terms change or
the exact per-file licence is clarified differently, this fixture must be
re-evaluated; the ambiguity is recorded in `fixtures/manifest` `known_limits`.

Upstream project: <https://github.com/qcad/qcad>. QCAD is a trademark of RibbonSoft.

## Human visual acceptance

`flange.png`/`flange.pdf` are references to compare against *by eye* (ignore
colour, account for viewport margins, line width and antialiasing). The
automated `scripts/check-dxf-reference.py` only asserts a non-empty frame, the
software adapter, and a coarse geometric **sanity** band; it does not and cannot
prove graphical correctness.
