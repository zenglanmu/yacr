# QCAD example DXF corpus — provenance

The DXF files in this directory are the upstream QCAD `examples/` drawings, used
as a **parse/render corpus** and regression input. They are committed because the
QCAD project distributes them openly (see "Licence" below). They are drawing
files, not user data.

## Origin

| Field | Value |
|---|---|
| Upstream project | QCAD — The Open Source 2D CAD (<https://github.com/qcad/qcad>) |
| Upstream path | `examples/` |
| Upstream commit at download | `dcf5754b0a19d8e57eddd467d18bf802ac12c2e2` (`master`, "cleanup", 2026-09-29) |
| Download URL pattern | `https://raw.githubusercontent.com/qcad/qcad/master/examples/<name>` |
| Downloaded | 2026-10-03, via the operator-provided HTTP proxy `http://192.168.8.1:10809` |
| Transport | `curl -fsSL` (no range requests, no transformation) |

The `flange.svg` vector reference lives with the other `qcad-flange` files.

## Files

| File | Bytes | SHA-256 |
|---|---|---|
| `calibration.dxf` | 100910 | `052f89afa57820f7c14ecf71adb4935ac98fcef1652f2188e22f95e1b11c6904` |
| `colors.dxf` | 51309 | `7f6b49efadf997d68f6ec23e6fea1ab5ca91e76ffeb73b63eb8e21440efc6766` |
| `entities.dxf` | 194947 | `fa0b296dc6c060ce8d79c61dd6461c9e1523a15ae33cdc650175ee8b9270d1d8` |
| `example00.dxf` | 158573 | `d6e89c4b74639a488cd5d6384a5aeeebaccafef138cb2a1a8c632cef0cdf1484` |
| `example01.dxf` | 84540 | `a816c31d7d9f661c941f6004d2b78861f8ff598b638fed7c8ec9e3fe54276c66` |
| `isometric_grid.dxf` | 106577 | `8cf9691a9d6eae38cce648b123e8e9878cbc4ca95e61051ba20f55fd11db1057` |
| `linetypes.dxf` | 51936 | `7f45ac1a1305ea724d1508eae5567840fc132244d5a405288b03b7454227848e` |
| `lineweights.dxf` | 53991 | `9ad30375df5b66afac4eb4f19f13e2b63fddf32c9847927d88071f0d29768f6a` |
| `projection.dxf` | 77857 | `11da9ea18f4c15a1bd34385d13b9346b747492692cc1cd0f6be3559b14a0eef5` |
| `../qcad-flange/flange.svg` | 399805 | `6489bc7dee9ee776e42e706e2e918784cb4a0e167ce07c3a96e5a28c15c72ee6` |

## Entity inventory

Entity types appearing in the corpus (model space plus block definitions), by
file. `scripts/check-qcad-examples.py` re-derives this and the render status.

| File | Entity types |
|---|---|
| `calibration.dxf` | LWPOLYLINE, VIEWPORT |
| `colors.dxf` | LWPOLYLINE, LINE, MTEXT |
| `entities.dxf` | LINE, MTEXT, INSERT, LWPOLYLINE, SPLINE, DIMENSION, ARC, CIRCLE, POINT, ELLIPSE, HATCH, LEADER, SOLID |
| `example00.dxf` | HATCH, LINE, ARC, SPLINE |
| `example01.dxf` | HATCH, LINE, DIMENSION, LEADER, MTEXT, INSERT, ARC, SOLID, POINT |
| `isometric_grid.dxf` | LINE, ELLIPSE, HATCH, DIMENSION, ARC, SOLID, MTEXT, POINT |
| `linetypes.dxf` | LWPOLYLINE, LINE, MTEXT |
| `lineweights.dxf` | LWPOLYLINE, LINE, MTEXT |
| `projection.dxf` | LINE, ELLIPSE, LWPOLYLINE, CIRCLE |

## Licence

QCAD's `LICENSE.txt` states that the QCAD 3 source code is distributed under
**GPL version 3 with optional exceptions**, and that *icons and documentation*
are distributed under **Creative Commons Attribution 3.0 Unported (CC BY 3.0)**.
The `examples/` files carry no separate per-file licence notice. This repository
is AGPL-3.0; GPLv3 and AGPLv3 are explicitly compatible (AGPLv3 §13 permits
combining with GPLv3), and CC BY 3.0 only requires attribution, which is given
here. Re-evaluate if upstream clarifies the per-file terms.

Upstream project: <https://github.com/qcad/qcad>. QCAD is a trademark of RibbonSoft.
