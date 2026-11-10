# RVT fixtures

Three committed fixtures, plus one local-only Autodesk sample that must never be
committed (see the last section). They are separated because no *redistributable*
file with real Revit geometry is available to this repository.

| File | Purpose | Authored by | SHA-256 |
|---|---|---|---|
| `synthetic-scaffold-2024.rvt` | Unit 1: `rvt` decode -> GLB | upstream `rvt` 0.17.0 `gen-fixture` | `da88093b42e740551402b0e0ed30b879b959e067d2044ff8bbc0d8d1553d8e9e` |
| `synthetic-two-node.glb` | Unit 2: GLB -> world-space mesh | this repository (`make-two-node-glb.py`) | `56ff8f501177528336db6d46a9ae7d5edd408261d2bc21d68535816814cf5d35` |
| `synthetic-materials.glb` | Unit 2: GLB material -> colour/opacity | this repository (`make-materials-glb.py`) | `cc0bc4b20762532f499eb9ba98c10bf9e327c5f30b36a5dab518da847a033ff5` |

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

## `synthetic-materials.glb`

Authored in this repository by `make-materials-glb.py`; regenerate with:

```text
python3 fixtures/rvt/make-materials-glb.py
```

Four square nodes, one mesh instance each, laid out along `x` so nothing can
hide at the origin:

| Node | `material` | What the contract asserts |
|---|---|---|
| `opaque` | 0: `baseColorFactor` = rvt's linearisation of sRGB `(200, 100, 50)`, alpha `1.0` | the reader converts linear -> sRGB byte-exactly (`[200, 100, 50]`) and stores no explicit opacity for alpha `1.0` |
| `blend` | 1: sRGB `(30, 144, 255)`, alpha `0.5`, `alphaMode: "BLEND"` | the reader stores `[30, 144, 255]` plus `EntityTransparency::Explicit(0.5)` (opacity, not a transparency fraction) |
| `plain` | none | the primitive stays `ByLayer`; no colour is invented |
| `conflict` | one mesh, two primitives, materials 0 and 1 | the merged entity keeps `ByLayer` (picking one material would recolour the other primitive's triangles) and the loss is reported as `import.rvt.material_ignored` |

The linear values are pinned literals equal to `rvt` 0.17.0's own
`colour_material` linearisation (`src/ifc/gltf.rs:665-675`) of those sRGB
bytes, so the fixture pins the exact inverse transfer function rather than a
`powf(1.0/2.2)` approximation. A separate test patches the JSON to an
out-of-range `baseColorFactor` (`1.5`) and asserts `ByLayer` plus the stable
diagnostic.

**No textures, by design.** `rvt` 0.17.0 emits no `images`/`textures`/
`samplers` and no `TEXCOORD_0` (`GltfDocument` has no such fields), and
`cad_domain::Mesh` carries only optional per-vertex `colors`, with no UV
channel. Texture mapping is therefore neither available upstream nor
representable in the stored geometry, not a missing feature of this reader.

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
