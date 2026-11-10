# IFC fixtures

All six files are **hand-authored in this repository** (`synthetic: true`,
`authorized: true`) and are ISO 10303-21 Part 21 text with an IFC `FILE_SCHEMA`.
They are contract fixtures for the import lane, not compatibility or
product-acceptance evidence, and they are not produced by any IFC authoring
tool.

They contain no vendor, commercial or personal data. No third-party IFC
certification sample is committed: the buildingSMART certification repository's
redistribution terms were not established here, so a real-file test is gated by
the `YACR_TEST_IFC` environment variable instead (see below) and a skip is **not**
evidence.

| File | Schema | Purpose | Expected |
|---|---|---|---|
| `synthetic-wall.ifc` | IFC4 | One `IfcWall`, 4000 × 200 × 3000 mm box extrusion; pins mm → m canonicalisation and one-entity-per-product | one entity, 12 triangles, `Complete` |
| `synthetic-wall-opening.ifc` | IFC4 | The same wall plus an `IfcOpeningElement` linked by `IfcRelVoidsElement`; pins opening subtraction and that the opening is not inserted | one entity, 32 triangles (12 uncut) |
| `synthetic-two-walls-ifc2x3.ifc` | IFC2X3 | Two `IfcWallStandardCase` products sharing one representation, placed at x=0 and x=+6000 mm; pins IFC2X3 parsing and world placement (no stacking at the origin) | two entities, 24 triangles, x 0..4 m and 6..10 m |
| `synthetic-wall-no-units.ifc` | IFC4 | The wall with no `IfcUnitAssignment`; pins the honest unknown-unit path | `Partial`, `import.ifc.units_unknown`, drawing units, raw file numbers |
| `synthetic-orphan-opening.ifc` | IFC4 | The wall plus an `IfcOpeningElement` that no `IfcRelVoidsElement` references; pins the orphan-cutter channel | one wall entity, `Partial`, `import.ifc.opening_without_host` |
| `synthetic-axis-only.ifc` | IFC4 | A wall whose only representation is an `Axis` `Curve2D`; pins the every-element-failed refusal | `Unsupported` (`import.ifc.all_elements_failed`) |

Every expected triangle count above is asserted by
`crates/cad-import-ifc/src/tests.rs`. A hand-written file that no longer parses
with `ifc-lite` fails those tests: the fixtures are themselves part of the
contract.

## Real buildingSMART files (not committed)

Set `YACR_TEST_IFC` to an absolute path of a real IFC file to run the opt-in
test (same convention as `YACR_TEST_DWG`). It is skipped when unset; **a skip is
not a run and not evidence**. The spike that selected the library parsed nine
real certification files (IFC2X3/IFC4/IFC4X3, 127–6,393 entities), but that was
a bounded local experiment, not a committed compatibility claim or product
acceptance.
