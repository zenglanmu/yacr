# synthetic-quad.obj

- **Provenance**: authored in this repository for the OBJ import contract tests.
- **Licence**: same as the repository (Apache-2.0 OR MIT).
- **Synthetic**: yes. No vendor, customer or third-party data.
- **Content**: one object (`synthetic-quad`) with four vertices, one face normal
  and one quad face. The reader fan-triangulates it into two triangles and marks
  the import `Partial` because n-gon fan triangulation is not exact for concave
  faces.
- **Known limits**: no material library, no units, no textures. This fixture
  pins the import contract only; it is not compatibility evidence.
