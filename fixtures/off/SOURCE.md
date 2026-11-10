# Synthetic OFF fixtures

- **Provenance**: authored in this repository. No vendor, customer or third-party
  data.
- **Licence**: same as the repository (Apache-2.0 OR MIT).
- **Synthetic**: yes. These fixtures pin the reader contract; they are not
  compatibility evidence.

## Files

- `synthetic-quad-triangle.off` — plain ASCII `OFF`. Five vertices, one quad and
  one triangle (three output triangles after fan triangulation).
- `synthetic-colors.coff` — ASCII `COFF` with unambiguous integer vertex colours
  (`0-255`). Four vertices and two triangular faces (two output triangles).

## Known limits

ASCII only; no units, materials, textures or face colours. The quad is
fan-triangulated, and concave n-gons are a known inexactness reported by the
reader. `COFF` colour values are integer `0-255`; integer `0/1` triples are
reported as ambiguous and dropped rather than guessed.
