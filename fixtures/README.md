# `fixtures/manifest` — CAD fixture provenance and expectations

Spec §11.5 / line 422 requires the repository to record, for every CAD fixture:
hash, source authorization, DWG version, producing software, fonts/xrefs, entity
types, expected support and known limits. This file is that record.

## Current state (honest)

**No authorized DWG, font or golden image is committed.** The `fixtures` list is
therefore the smallest honest form: one **synthetic** contract fixture, plus the
notice explaining why the rest is absent. Passing tests over synthetic bytes does
**not** establish compatibility with AutoCAD, Tianzheng (天正), TSSD (探索者) or
any vendor format, and the manifest enforces that.

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

## Authorizing a real sample

A real drawing or vendor file may only be added with a verified licence. Record
it by filling `provenance` (origin, authorization, licence), setting
`authorized: true`, `synthetic: false`, and an `expected` value you can defend
with evidence. Do **not** commit user drawings, and never upload a user drawing
to public CI. See also `docs/compatibility.md` and `docs/validation.md`.
