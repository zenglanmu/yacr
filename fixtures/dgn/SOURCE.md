# DGN fixtures

Public GDAL/OGR test data, fetched from `OSGeo/gdal` at commit
`ccd38119ca19b6b656eed52fc5c83699d4ce89bf` (master, 2026-11-14):

- `smalltest.dgn` — `autotest/ogr/data/dgn/smalltest.dgn` (V7 ISFF, 2D).
- `test_dgnv8.dgn` — `autotest/ogr/data/dgnv8/test_dgnv8.dgn` (V8 OLE/CFB).
- `test_dgnv8_ref.csv` — `autotest/ogr/data/dgnv8/test_dgnv8_ref.csv`, the ODA
  driver's expected output for the V8 sample (reference only; the importer does
  not read it).
- `GDAL-LICENSE.TXT` — GDAL/OGR's MIT-style licence, copied verbatim.

SHA-256 (also recorded in `fixtures/manifest`):

```
1dae3468e81d00da56e2936f74d33b8b3ad09d726437f19ce209a5dabea41f77  GDAL-LICENSE.TXT
9d9faddb67216f9d56fc9a1027adc1a927c8c13529dfd3416b771b4dc4e9a284  smalltest.dgn
8f32f87ce4b16881aa64f5cb9f75c98851833f96fef37ca0ad31aa6bb18d1df0  test_dgnv8.dgn
09f765e0aa8dc06bb1d0da4ed86f060dc0a839d480b3dc67e54841ecceec6433  test_dgnv8_ref.csv
```

## Evidence boundary

These are contract fixtures, not Bentley/MicroStation acceptance evidence:

- `test_dgnv8.dgn` was written by **ODA Teigha DGN 4.02**, not MicroStation.
  It is a 27 KB toy with 35 top-level entities, one shared-cell definition and
  two unsupported element types.
- `smalltest.dgn` is the long-standing GDAL V7 sample; it exercises four
  graphics on two levels and no cells or rasters.
- Neither file proves V8i/CONNECT/ORD compatibility, level display flags,
  references, Item Types, dimensions, multiline elements, B-spline surface
  bodies, or raster loading. Those remain explicitly unsupported/Partial in the
  importer and are documented in `docs/dgn.md`.
- Real MicroStation files may be supplied out-of-tree through
  `YACR_TEST_DGN=<absolute path>`; a skipped run is **not** evidence.
