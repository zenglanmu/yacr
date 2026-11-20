#!/usr/bin/env python3
"""Synthesise a glTF 2.0 sample that exercises every PBR channel.

Geometry: one 1 m quad facing +Y in the glTF frame (so it becomes the CAD XY
plane after the importer's +Y-up -> +Z-up mapping), UV0 covering 0..1.

Textures (all embedded in the BIN chunk, no external files):

* baseColor      four quadrants: red / green / blue / grey
* metallicRough  glTF packs roughness in G and metallic in B; left half is a
                 rough dielectric, right half a smooth metal
* normal          flat for the left half, tilted for the right half
* emissive        black except a small bright-green square in the middle
* occlusion      white except a dark band at the bottom edge

Run: ``make_pbr_gltf.py <outdir>`` writes ``pbr-channels.glb``.
"""
import json
import struct
import sys
import zlib

W = H = 64


def _png(rgb_of) -> bytes:
    raw = bytearray()
    for y in range(H):
        raw.append(0)
        for x in range(W):
            raw.extend(rgb_of(x, y))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def base_color(x, y):
    left, top = x < W // 2, y < H // 2
    return {
        (True, True): (210, 40, 40),
        (False, True): (40, 170, 60),
        (True, False): (50, 70, 200),
        (False, False): (150, 150, 150),
    }[(left, top)]


def metallic_rough(x, y):
    # G = roughness, B = metallic. Left: rough dielectric. Right: smooth metal.
    if x < W // 2:
        return (0, 200, 0)
    return (0, 20, 255)


def normal(x, y):
    # Flat (0.5,0.5,1) on the left; tilted towards +U on the right.
    if x < W // 2:
        return (128, 128, 255)
    return (200, 110, 190)


def emissive(x, y):
    if 26 <= x < 38 and 26 <= y < 38:
        return (0, 255, 0)
    return (0, 0, 0)


def occlusion(x, y):
    if y >= H - 8:
        return (20, 20, 20)
    return (255, 255, 255)


def build() -> bytes:
    # Quad in the glTF XZ plane; the importer maps (x,y,z) -> (x,-z,y).
    positions = [
        (-0.5, 0.0, 0.5),
        (0.5, 0.0, 0.5),
        (0.5, 0.0, -0.5),
        (-0.5, 0.0, -0.5),
    ]
    uvs = [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)]
    indices = [0, 1, 2, 0, 2, 3]

    blob = bytearray()
    views = []

    def add_view(data: bytes, target=None):
        while len(blob) % 4:
            blob.append(0)
        view = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)}
        if target is not None:
            view["target"] = target
        views.append(view)
        blob.extend(data)
        return len(views) - 1

    pos = b"".join(struct.pack("<3f", *p) for p in positions)
    uv = b"".join(struct.pack("<2f", *t) for t in uvs)
    idx = b"".join(struct.pack("<I", i) for i in indices)
    images = [base_color, metallic_rough, normal, emissive, occlusion]
    image_views = [add_view(_png(fn)) for fn in images]

    v_pos = add_view(pos, 34962)
    v_uv = add_view(uv, 34962)
    v_idx = add_view(idx, 34963)

    accessors = [
        {
            "bufferView": v_pos,
            "componentType": 5126,
            "count": len(positions),
            "type": "VEC3",
            "min": [-0.5, 0.0, -0.5],
            "max": [0.5, 0.0, 0.5],
        },
        {"bufferView": v_uv, "componentType": 5126, "count": len(uvs), "type": "VEC2"},
        {"bufferView": v_idx, "componentType": 5125, "count": len(indices), "type": "SCALAR"},
    ]

    while len(blob) % 4:
        blob.append(0)

    doc = {
        "asset": {"version": "2.0", "generator": "yacr-e2e-pbr"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "name": "pbr-quad"}],
        "meshes": [
            {
                "name": "pbr-quad",
                "primitives": [
                    {
                        "attributes": {"POSITION": 0, "TEXCOORD_0": 1},
                        "indices": 2,
                        "mode": 4,
                        "material": 0,
                    }
                ],
            }
        ],
        "images": [{"bufferView": v, "mimeType": "image/png"} for v in image_views],
        "samplers": [
            {"magFilter": 9729, "minFilter": 9729, "wrapS": 10497, "wrapT": 10497}
        ],
        "textures": [{"sampler": 0, "source": i} for i in range(len(images))],
        "materials": [
            {
                "name": "pbr-all-channels",
                "pbrMetallicRoughness": {
                    "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
                    "baseColorTexture": {"index": 0, "texCoord": 0},
                    "metallicFactor": 1.0,
                    "roughnessFactor": 1.0,
                    "metallicRoughnessTexture": {"index": 1, "texCoord": 0},
                },
                "normalTexture": {"index": 2, "texCoord": 0, "scale": 1.0},
                "occlusionTexture": {"index": 4, "texCoord": 0, "strength": 1.0},
                "emissiveTexture": {"index": 3, "texCoord": 0},
                "emissiveFactor": [1.0, 1.0, 1.0],
            }
        ],
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": len(blob)}],
    }

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
    with open(f"{target}/pbr-channels.glb", "wb") as f:
        f.write(build())
    print("wrote pbr-channels.glb")
