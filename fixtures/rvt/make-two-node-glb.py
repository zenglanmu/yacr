#!/usr/bin/env python3
"""Author `synthetic-two-node.glb`, the unit-2 contract fixture.

Not a CAD sample: a minimal glTF 2.0 binary with one square mesh instanced by
two nodes that must end up at different world positions.

* `block-a` carries a **matrix** translation of x = -1.
* `group` (no mesh) translates x = +0.5 and parents `block-b`, whose local
  translation is x = +0.5, so `block-b`'s world translation is x = +1.

A reader that ignores node transforms stacks both squares at the origin. A
reader that ignores parent composition puts them at -1 and +0.5. Only the full
`parent_world * local` chain puts them at -1 and +1.

The mesh lies in glTF's XZ plane (y = 0) with +Y normals. This importer maps
glTF +Y up to CAD +Z up, so the imported squares lie in the CAD XY plane at
z = 0 and their normals are (0, 0, 1).

Coordinates are glTF metres (the specification's unit); the fixture does not
depend on Revit or on `rvt`.

Run from the repository root:

    python3 fixtures/rvt/make-two-node-glb.py
"""

from __future__ import annotations

import json
import pathlib
import struct

ROOT = pathlib.Path(__file__).resolve().parent
OUTPUT = ROOT / "synthetic-two-node.glb"

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

DOCUMENT = {
    "asset": {"version": "2.0", "generator": "yacr fixtures/rvt/make-two-node-glb.py"},
    "scene": 0,
    "scenes": [{"nodes": [0, 1]}],
    "nodes": [
        {
            "name": "block-a",
            "mesh": 0,
            # Column-major: last column is the translation (-1, 0, 0).
            "matrix": [
                1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                -1.0, 0.0, 0.0, 1.0,
            ],
        },
        {"name": "group", "translation": [0.5, 0.0, 0.0], "children": [2]},
        {"name": "block-b", "mesh": 0, "translation": [0.5, 0.0, 0.0]},
    ],
    "meshes": [
        {
            "name": "square",
            "primitives": [
                {
                    "attributes": {"POSITION": 0, "NORMAL": 1},
                    "indices": 2,
                    "mode": 4,
                }
            ],
        }
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


def main() -> None:
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
