# Synthetic PLY fixtures

- **Provenance**: authored in this repository. No vendor, customer or third-party
  data.
- **Licence**: same as the repository (Apache-2.0 OR MIT).
- **Synthetic**: yes. These fixtures pin the reader contract; they are not
  compatibility evidence.

## Files

- `synthetic-attrs-ascii.ply` — ASCII PLY 1.0. Five vertices, one quad and one
  triangle (three output triangles after fan triangulation). The vertex property
  order is deliberately non-standard (`red, z, x, blue, y, nx, ny, nz, green`)
  so a reader that assumes `x,y,z` are the first three properties fails.
  Normals `(0,0,1)` and uchar colours are present on every vertex.
- `synthetic-binary-le.ply` — the same mesh in `binary_little_endian 1.0` with
  standard property order `x,y,z,nx,ny,nz,red,green,blue`.
- `synthetic-binary-be.ply` — the same mesh in `binary_big_endian 1.0`.

The two binary files were produced by this `struct` script (run from the
repository root); the ASCII file is the hand-authored text above.

```python
import struct

verts = [
    (0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 255, 0, 0),
    (1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0, 255, 0),
    (1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0, 0, 255),
    (0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 255, 255, 0),
    (2.0, 1.0, 0.0, 0.0, 0.0, 1.0, 255, 0, 255),
]
faces = [[0, 1, 2, 3], [0, 2, 4]]

def header(endian):
    encoding = "binary_little_endian" if endian == "<" else "binary_big_endian"
    return (
        "ply\n"
        f"format {encoding} 1.0\n"
        "comment synthetic fixture authored in this repository; same mesh as synthetic-attrs-ascii.ply\n"
        "element vertex 5\n"
        "property float x\nproperty float y\nproperty float z\n"
        "property float nx\nproperty float ny\nproperty float nz\n"
        "property uchar red\nproperty uchar green\nproperty uchar blue\n"
        "element face 2\nproperty list uchar int vertex_indices\nend_header\n"
    ).encode("ascii")

def payload(endian):
    out = bytearray()
    for v in verts:
        out += struct.pack(endian + "6f", *v[:6])
        out += bytes(v[6:9])
    for face in faces:
        out += struct.pack("B", len(face))
        out += struct.pack(endian + f"{len(face)}i", *face)
    return bytes(out)

for endian, name in [("<", "synthetic-binary-le.ply"), (">", "synthetic-binary-be.ply")]:
    with open("fixtures/ply/" + name, "wb") as f:
        f.write(header(endian) + payload(endian))
```

## Known limits

No units, no materials, no textures, no edge/other elements. The quad is
fan-triangulated, and concave n-gons are a known inexactness reported by the
reader. These files only exercise the parser and mesh invariants.
