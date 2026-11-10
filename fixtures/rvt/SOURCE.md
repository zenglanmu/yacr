# RVT fixtures

Two committed fixtures, plus one local-only Autodesk sample that must never be
committed (see the last section). They are separated because no *redistributable*
file with real Revit geometry is available to this repository.

| File | Purpose | Authored by | SHA-256 |
|---|---|---|---|
| `synthetic-scaffold-2024.rvt` | Unit 1: `rvt` decode -> GLB | upstream `rvt` 0.17.0 `gen-fixture` | `da88093b42e740551402b0e0ed30b879b959e067d2044ff8bbc0d8d1553d8e9e` |
| `synthetic-two-node.glb` | Unit 2: GLB -> world-space mesh | this repository (`make-two-node-glb.py`) | `56ff8f501177528336db6d46a9ae7d5edd408261d2bc21d68535816814cf5d35` |

## `synthetic-scaffold-2024.rvt`

Generated with the upstream `rvt` 0.17.0 `gen-fixture` binary, which the crate
ships as `src/bin/gen_fixture.rs`. It is an OLE/CFB container with the twelve
invariant Revit streams and one `Partitions/67` stream; **its stream contents
are hand-built by that tool**, not captured from Revit. Its own module
documentation says the file "will NOT open in Revit" and is meant to exercise
the reader/walker/compression layers.

Provenance: upstream <https://github.com/DrunkOnJava/rvt-rs>, crate `rvt`
0.17.0 (published 2026-11-09), Apache-2.0. No Autodesk data or Revit
installation was involved. Redistribution is covered by the crate's Apache-2.0
licence; the file is copied as generated, unmodified.

Exact generation command (run in a scratch crate that depends on `rvt =
"=0.17.0"` and reuses the upstream binary):

```text
gen_fixture synthetic-scaffold-2024 --year 2024 \
  --classes Wall,Level,Project --element-count 10 \
  --output fixtures/rvt/synthetic-scaffold-2024.rvt
```

The tool is deterministic for the same name/seed/classes/count/year.

**What it proves — and what it does not.** `rvt` 0.17.0 decodes it to
`confidence.level = "scaffold"`, `building_elements = 0`,
`building_elements_with_geometry = 0`, `confidence.has_geometry = false`, and a
GLB with one root node and no meshes. The contract test asserts that honest
zero and the importer's explicit `import.rvt.no_geometry` refusal. It is not a
Revit model, not compatibility evidence, and never a successful import.

The stream contents, including `BasicFileInfo`'s synthetic save path (which
ends in `.rfa`), are the upstream tool's placeholders. The importer classifies
families from the `ProjectInformation` stream and the `PartAtom` family table,
not from that path; this fixture has neither a `ProjectInformation` stream nor
a family table, so it is decoded (and refused for having no geometry), not
misreported as a family.

## `synthetic-two-node.glb`

Authored in this repository by `make-two-node-glb.py`; regenerate with:

```text
python3 fixtures/rvt/make-two-node-glb.py
```

One square mesh (four vertices, two indexed triangles, `+Y` normals) instanced
twice:

* `block-a` uses a **matrix** translation of `x = -1`;
* `group` (no mesh) translates `x = +0.5` and parents `block-b`, whose local
  translation is `x = +0.5` (world `x = +1`).

The importer applies `parent_world * local` and maps glTF `+Y` up to CAD `+Z`
up. The test asserts the two world bounding boxes are `x ∈ [-1.5, -0.5]` and
`x ∈ [0.5, 1.5]` — non-overlapping. A reader that ignores node transforms puts
both squares at the origin, and one that ignores parent composition puts them at
`-1` and `+0.5`; either fails the assertion.

glTF is an **internal intermediate** of the RVT lane: it is not a user-visible
format, is not in detection or pickers, and has no capability row. The file is
a unit-2 contract fixture only, not CAD compatibility evidence.

## Local-only: `samples/racbasicsampleproject.rvt` (never commit)

| | |
|---|---|
| Source | <https://revit.downloads.autodesk.com/download/2024RVT_RTM/Docs/InProd/racbasicsampleproject.rvt> |
| Size | 19,722,240 bytes |
| SHA-256 | `bb62c161b7991f0c5ec76605d3d9d0407e04001a2e4e6acac1c7fc32fd45ee55` |
| Container | OLE/CFB (`D0 CF 11 E0 A1 B1 1A E1`), Revit 2024 sample project |
| Licence | Autodesk sample content; **redistribution not permitted** |

It is excluded by `.gitignore` (`fixtures/rvt/samples/`), has **no**
`fixtures/manifest` entry, and is never packaged. It exists to produce local
evidence for the preview importer on a real model. Any result obtained from it is
**local only / not shared** — it cannot be reproduced by anyone who does not
download the file from Autodesk, so it must not be cited as repository evidence
or as a compatibility claim.

Re-download with:

```text
mkdir -p fixtures/rvt/samples
curl -fL -o fixtures/rvt/samples/racbasicsampleproject.rvt \
  https://revit.downloads.autodesk.com/download/2024RVT_RTM/Docs/InProd/racbasicsampleproject.rvt
sha256sum fixtures/rvt/samples/racbasicsampleproject.rvt   # expect bb62c161…
```
