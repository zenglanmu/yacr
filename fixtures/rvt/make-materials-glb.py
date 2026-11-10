#!/usr/bin/env python3
"""Author `synthetic-materials.glb`, the material-colour contract fixture.

Not a CAD sample: a minimal glTF 2.0 binary that pins how the RVT preview lane
reads `pbrMetallicRoughness.baseColorFactor` and alpha. Four nodes, one square
each, laid out along x so nothing can hide at the origin:

* `opaque` — material 0, an opaque colour. `rvt` linearises sRGB bytes into
  `baseColorFactor` (`rvt` `src/ifc/gltf.rs:665-675`), so the linear values
  below are exactly that linearisation of sRGB `(200, 100, 50)` and must decode
  back to those bytes;
* `blend`  — material 1, sRGB `(30, 144, 255)` with `alpha = 0.5` and
  `alphaMode = "BLEND"`, so the reader stores an explicit opacity;
* `plain`  — a primitive with **no** `material`, which must stay `ByLayer`
  rather than acquire an invented colour;
* `conflict` — one mesh with **two** primitives naming materials 0 and 1. The
  reader merges a node's primitives into one entity, so it must keep `ByLayer`
  and report the disagreement instead of silently picking one material.

`rvt` emits no textures at all (no `images`/`textures`/`samplers`, no
`TEXCOORD_0`), and `cad_domain::Mesh` has no UV channel; this fixture therefore
carries none, by design.

Run from the repository root:

    python3 fixtures/rvt/make-materials-glb.py
"""

from __future__ import annotations

import json
import pathlib
import struct

ROOT = pathlib.Path(__file__).resolve().parent
OUTPUT = ROOT / "synthetic-materials.glb"

# One square, four vertices, two triangles, in glTF's XZ plane.
VERTICES = [
    (-0.5, 0.0, -0.5),
    (0.5, 0.0, -0.5),
    (0.5, 0.0, 0.5),
    (-0.5, 0.0, 0.5),
]
NORMALS = [(0.0, 1.0, 0.0)] * 4
INDICES = [0, 1, 2, 0, 2, 3]

POSITION_BYTES = b"".join(struct.pack("<3f", *vertex) for vertex in VERTICES)
NORMAL_BYTES = b"".join(struct.pack("<3f", *normal) for normal in NORMALS)
INDEX_BYTES = b"".join(struct.pack("<H", index) for index in INDICES)
BIN = POSITION_BYTES + NORMAL_BYTES + INDEX_BYTES

POSITION_OFFSET = 0
NORMAL_OFFSET = len(POSITION_BYTES)
INDEX_OFFSET = NORMAL_OFFSET + len(NORMAL_BYTES)

# sRGB bytes and the exact linear `baseColorFactor` `rvt` would write for them
# (computed once, then pinned as literals so regeneration is byte-stable).
OPAQUE_SRGB = (200, 100, 50)
OPAQUE_LINEAR = [0.577580451965332, 0.12743768095970154, 0.03189603239297867]
BLEND_SRGB = (30, 144, 255)
BLEND_LINEAR = [0.012983032502233982, 0.2788942754268646, 1.0]
BLEND_ALPHA = 0.5


def srgb_to_linear(byte: int) -> float:
    """Reference linearisation, used only to check the pinned literals."""
    value = struct.unpack("<f", struct.pack("<f", byte / 255.0))[0]
    if value <= 0.04045:
        return value / 12.92
    return ((value + 0.055) / 1.055) ** 2.4


DOCUMENT = {
    "asset": {"version": "2.0", "generator": "yacr fixtures/rvt/make-materials-glb.py"},
    "scene": 0,
    "scenes": [{"nodes": [0, 1, 2, 3]}],
    "nodes": [
        {"name": "opaque", "mesh": 0, "translation": [-1.5, 0.0, 0.0]},
        {"name": "blend", "mesh": 1, "translation": [0.0, 0.0, 0.0]},
        {"name": "plain", "mesh": 2, "translation": [1.5, 0.0, 0.0]},
        {"name": "conflict", "mesh": 3, "translation": [3.0, 0.0, 0.0]},
    ],
    "meshes": [
        {
            "name": "opaque-square",
            "primitives": [
                {"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 0, "mode": 4}
            ],
        },
        {
            "name": "blend-square",
            "primitives": [
                {"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 1, "mode": 4}
            ],
        },
        {
            "name": "plain-square",
            "primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "mode": 4}],
        },
        {
            "name": "conflict-square",
            "primitives": [
                {"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 0, "mode": 4},
                {"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 1, "mode": 4},
            ],
        },
    ],
    "materials": [
        {
            "name": "Opaque 200/100/50",
            "pbrMetallicRoughness": {
                "baseColorFactor": [*OPAQUE_LINEAR, 1.0],
                "metallicFactor": 0.0,
                "roughnessFactor": 0.6,
            },
            "doubleSided": False,
        },
        {
            "name": "Blend 30/144/255 alpha 0.5",
            "pbrMetallicRoughness": {
                "baseColorFactor": [*BLEND_LINEAR, BLEND_ALPHA],
                "metallicFactor": 0.0,
                "roughnessFactor": 0.6,
            },
            "doubleSided": True,
            "alphaMode": "BLEND",
        },
    ],
    "buffers": [{"byteLength": len(BIN)}],
    "bufferViews": [
        {"buffer": 0, "byteOffset": POSITION_OFFSET, "byteLength": len(POSITION_BYTES)},
        {"buffer": 0, "byteOffset": NORMAL_OFFSET, "byteLength": len(NORMAL_BYTES)},
        {"buffer": 0, "byteOffset": INDEX_OFFSET, "byteLength": len(INDEX_BYTES)},
    ],
    "accessors": [
        {
            "bufferView": 0,
            "byteOffset": 0,
            "componentType": 5126,
            "count": len(VERTICES),
            "type": "VEC3",
            "min": [-0.5, 0.0, -0.5],
            "max": [0.5, 0.0, 0.5],
        },
        {
            "bufferView": 1,
            "byteOffset": 0,
            "componentType": 5126,
            "count": len(NORMALS),
            "type": "VEC3",
        },
        {
            "bufferView": 2,
            "byteOffset": 0,
            "componentType": 5123,
            "count": len(INDICES),
            "type": "SCALAR",
        },
    ],
}


def pad(data: bytes, fill: bytes) -> bytes:
    while len(data) % 4:
        data += fill
    return data


def check_pinned_values() -> None:
    """The pinned literals are the same linear values within f32 rounding."""
    for byte, linear in zip(OPAQUE_SRGB, OPAQUE_LINEAR):
        expected = struct.unpack("<f", struct.pack("<f", srgb_to_linear(byte)))[0]
        assert abs(expected - linear) < 1e-6, (byte, expected, linear)
    for byte, linear in zip(BLEND_SRGB, BLEND_LINEAR):
        expected = struct.unpack("<f", struct.pack("<f", srgb_to_linear(byte)))[0]
        assert abs(expected - linear) < 1e-6, (byte, expected, linear)


def main() -> None:
    check_pinned_values()
    json_bytes = pad(json.dumps(DOCUMENT, separators=(",", ":")).encode("utf-8"), b" ")
    bin_bytes = pad(BIN, b"\x00")
    total = 12 + 8 + len(json_bytes) + 8 + len(bin_bytes)
    glb = bytearray()
    glb += struct.pack("<4sII", b"glTF", 2, total)
    glb += struct.pack("<II", len(json_bytes), 0x4E4F534A)
    glb += json_bytes
    glb += struct.pack("<II", len(bin_bytes), 0x004E4942)
    glb += bin_bytes
    OUTPUT.write_bytes(glb)
    print(f"wrote {OUTPUT} ({len(glb)} bytes)")


if __name__ == "__main__":
    main()
