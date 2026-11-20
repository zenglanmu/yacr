#!/usr/bin/env python3
"""Synthesise textured / untextured GLB samples for end-to-end checks.

Builds a 2-triangle quad in the glTF frame (metres, +Y up) whose UV0 maps a
four-quadrant colour checker (red / green / blue / yellow). The PNG is embedded
in the BIN chunk through a bufferView, i.e. exactly the shape our importer
supports without any external file.
"""
import json
import struct
import sys
import zlib

W = H = 64


def png_bytes() -> bytes:
    quadrants = {
        (0, 0): (220, 30, 30),
        (1, 0): (30, 180, 60),
        (0, 1): (40, 60, 220),
        (1, 1): (230, 200, 40),
    }
    raw = bytearray()
    for y in range(H):
        raw.append(0)  # filter: none
        for x in range(W):
            q = (0 if x < W // 2 else 1, 0 if y < H // 2 else 1)
            raw.extend(quadrants[q])

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0)  # 8-bit RGB
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def build(textured: bool) -> bytes:
    # Quad in the glTF XZ plane (+Y is "up", so a floor plane). The importer maps
    # glTF (x, y, z) -> CAD (x, -z, y), so this becomes the CAD XY plane, i.e.
    # facing the default top view with a +Z normal.
    positions = [
        (-0.5, 0.0, 0.5),
        (0.5, 0.0, 0.5),
        (0.5, 0.0, -0.5),
        (-0.5, 0.0, -0.5),
    ]
    uvs = [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)]
    indices = [0, 1, 2, 0, 2, 3]

    pos = b"".join(struct.pack("<3f", *p) for p in positions)
    uv = b"".join(struct.pack("<2f", *t) for t in uvs)
    idx = b"".join(struct.pack("<I", i) for i in indices)
    image = png_bytes() if textured else b""

    views = []
    blob = bytearray()

    def add_view(data: bytes, target=None):
        while len(blob) % 4:
            blob.append(0)
        view = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)}
        if target is not None:
            view["target"] = target
        views.append(view)
        blob.extend(data)
        return len(views) - 1

    v_pos = add_view(pos, 34962)
    v_uv = add_view(uv, 34962)
    v_idx = add_view(idx, 34963)
    v_img = add_view(image) if textured else None

    accessors = [
        {
            "bufferView": v_pos,
            "componentType": 5126,
            "count": len(positions),
            "type": "VEC3",
            "min": [-0.5, 0.0, -0.5],
            "max": [0.5, 0.0, 0.5],
        },
        {
            "bufferView": v_uv,
            "componentType": 5126,
            "count": len(uvs),
            "type": "VEC2",
        },
        {
            "bufferView": v_idx,
            "componentType": 5125,
            "count": len(indices),
            "type": "SCALAR",
        },
    ]

    attributes = {"POSITION": 0}
    primitive = {"attributes": attributes, "indices": 2, "mode": 4}
    doc = {}
    if textured:
        attributes["TEXCOORD_0"] = 1
        doc["images"] = [{"bufferView": v_img, "mimeType": "image/png"}]
        doc["samplers"] = [
            {"magFilter": 9729, "minFilter": 9729, "wrapS": 10497, "wrapT": 10497}
        ]
        doc["textures"] = [{"sampler": 0, "source": 0}]
        doc["materials"] = [
            {
                "name": "checker",
                "pbrMetallicRoughness": {
                    "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
                    "baseColorTexture": {"index": 0, "texCoord": 0},
                },
            }
        ]
        primitive["material"] = 0

    while len(blob) % 4:
        blob.append(0)

    doc.update(
        {
            "asset": {"version": "2.0", "generator": "yacr-e2e"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0, "name": "quad"}],
            "meshes": [{"name": "quad", "primitives": [primitive]}],
            "accessors": accessors,
            "bufferViews": views,
            "buffers": [{"byteLength": len(blob)}],
        }
    )

    js = json.dumps(doc, separators=(",", ":")).encode()
    while len(js) % 4:
        js += b" "

    total = 12 + 8 + len(js) + 8 + len(blob)
    out = bytearray()
    out += b"glTF" + struct.pack("<II", 2, total)
    out += struct.pack("<I", len(js)) + b"JSON" + js
    out += struct.pack("<I", len(blob)) + b"BIN\x00" + bytes(blob)
    return bytes(out)


if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else "."
    with open(f"{target}/checker-textured.glb", "wb") as f:
        f.write(build(True))
    with open(f"{target}/checker-plain.glb", "wb") as f:
        f.write(build(False))
    print("wrote checker-textured.glb and checker-plain.glb")
