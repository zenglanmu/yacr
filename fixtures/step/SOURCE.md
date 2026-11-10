# STEP fixtures

All four files are ISO 10303-21 (Part 21) text. They exercise the tessellation
contract only; they are **not** compatibility or product-acceptance evidence.

## Vendored from cadmpeg (third party)

`ap214-sheet.p21` and `ap203-sheet.p21` are byte-for-byte copies of the upstream
test fixtures:

- crate: `cadmpeg-codec-step` 0.6.11
- local path in the crates.io package:
  `tests/fixtures/ap214_sheet.p21`, `tests/fixtures/ap203_sheet.p21`
- upstream: <https://github.com/cadmpeg/cadmpeg>
- licence: Apache-2.0 (the crate's declared licence; the files carry no separate
  per-file header)
- unmodified, third party (`synthetic: false`, `authorized: true`)

| File | SHA-256 | Expected import |
|---|---|---|
| `ap214-sheet.p21` | `386b95e5abf75a121aac57546cae90fda35d7408e38ee7650e3407268d24f2ed` | one open-shell triangular face tessellates to 1 triangle; `Partial` (open shell, unresolved plane-angle unit) |
| `ap203-sheet.p21` | `6b413e93728e96c93d6adde4d7a43a3dd5b181627bdbe28f20859891d5a78f62` | its only face is refused (`the edge … has no parameter range`); the import is an explicit `Unsupported`, never an empty success |

## Authored in this repository

| File | SHA-256 | Purpose |
|---|---|---|
| `synthetic-two-part-assembly.p21` | `84023742423fb7053e38ef5af2627bbaa54a1c6fe78e3735dc7ad72c8fe775f8` | AP214 assembly: two child product definitions, each one open-shell triangular sheet, placed by `NEXT_ASSEMBLY_USAGE_OCCURRENCE` + `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` at x=0 and x=+100 mm. Pins occurrence placement (bodies must not stack at the origin). |
| `synthetic-partial-refusal.p21` | `5cc0fb90727e9625115c1ebfed1703cf1477729c594b7c46b87d09e1cd5eaa97` | AP214 sheet with a second face appended: one face tessellates, one is refused. Pins `Partial` + `import.step.face_refused`. The refused-face records are derived from `ap203_sheet.p21`, so that portion remains Apache-2.0 cadmpeg material; the assembly is authored here. |

Both synthetic files are `authorized: true`, `synthetic: true`, and expected to
be `partial` (surface model / open shell, not a solid). They contain no vendor
or private data.
