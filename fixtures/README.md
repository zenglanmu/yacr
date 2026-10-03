# `fixtures/manifest` — CAD fixture provenance and expectations

Spec §11.5 / line 422 requires the repository to record, for every CAD fixture:
hash, source authorization, DWG version, producing software, fonts/xrefs, entity
types, expected support and known limits. This file is that record.

## Current state (honest)

Most entries are **synthetic** contract fixtures. In addition, one openly
distributed third-party sample is committed: the QCAD `flange` DXF with its
upstream PNG/PDF reference (`fixtures/dxf/qcad-flange/`, provenance in
`SOURCE.md`). Its reference images are **human-review aids and coarse sanity
checks, not project golden images**; the DXF still imports as `Partial` because
text drawing needs a host font, but its block-less dimensions are synthesized
and render (lines, arrowheads, measurement text). No user/vendor drawing or
third-party font is committed; reference images are only committed when the
upstream licence permits redistribution.

Passing tests over synthetic bytes does **not** establish compatibility with
AutoCAD, Tianzheng (天正), TSSD (探索者) or any vendor format, and the manifest
enforces that; importing the one QCAD sample is likewise not general DXF
compatibility evidence.

## Format

```json
{
  "schema_version": 1,
  "fixtures": [
    {
      "id": "synthetic-proxy-known-filloff-text",
      "path": "fixtures/proxy/known_filloff_text.hex",
      "sha256": "3e0d42facbc67a654a58b6efab535fad45aef60fe3ca1af0c54ba8d8c2a8374a",
      "dwg_version": "n/a",
      "source": "hand-authored from the documented proxy record layout",
      "provenance": {
        "origin": "yacr repository",
        "authorization": "synthetic; authored for this repository, no vendor data",
        "licence": "same as the repository"
      },
      "authorized": true,
      "synthetic": true,
      "entity_types": [],
      "expected": "partial",
      "known_limits": ["..."]
    }
  ],
  "notice": "..."
}
```

Required keys per entry: `id`, `path`, `sha256`, `dwg_version`, `source`,
`provenance`, `authorized`, `synthetic`, `entity_types`, `expected`,
`known_limits`. `sha256` must be 64 lower-case hex characters; a repository
`path` must exist.

## Rules the validator enforces

`scripts/check-fixture-manifest.py` (stdlib only, run in CI) fails when:

- the file is missing, is not JSON, or `schema_version` is not `1`;
- an entry lacks a required key, has a malformed `sha256`, or a repository
  `path` that does not exist;
- two entries share an `id`;
- a fixture that is `synthetic` **or** `authorized: false` claims
  `expected` of `complete`/`verified`/`supported` — an unbacked compatibility
  claim.

## Authorizing and committing a sample

A drawing or reference image may be committed when its licence clearly permits
redistribution. Record `provenance` (origin URL, authorization, licence), the
download date and the SHA-256, set `authorized: true`, `synthetic: false`, and an
`expected` value you can defend with evidence. Reference images produced by the
upstream project may be committed alongside the drawing as human-review aids.
Do **not** commit material whose redistribution you cannot justify, and never
upload private user data. See also `docs/compatibility.md` and
`docs/validation.md`.
