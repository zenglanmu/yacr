# `@yacr/plugin-api`

The published contract **and** host-agnostic reference client for YACR's
secondary-development (plugin) API, mirroring the AutoCAD .NET API namespaces in
idiomatic TypeScript. There are no host bindings and no CAD logic: no dependencies,
no DOM access, no Tauri import, no wasm glue. A carrier supplies a `CadHostBridge`;
the capability gate, transaction bookkeeping and value helpers are plain TypeScript.
The package is `private` and consumed from this repository, not from a registry.

```ts
import { definePlugin } from '@yacr/plugin-api';

export default definePlugin({
  id: 'sample-hello',
  apiVersion: 1,
  activate({ api }) {
    const p = api.geometry.point3d(0, 0, 0);
    return api.editor.writeMessage(`ready at ${p.x},${p.y}`);
  },
});
```

## Namespace map

| AutoCAD .NET namespace | This package |
| --- | --- |
| `ApplicationServices` | `Application`, `DocumentManager`, `Document` |
| `DatabaseServices` | `Database`, `Transaction`, `TransactionManager`, object/table descriptions |
| `EditorInput` | `Editor`, `PromptResult`, `PromptStatus`, `SelectionSet` |
| `Runtime` | `definePlugin`, `parsePluginManifest`, `PluginContext` |
| `Geometry` | `Point2d/3d`, `Vector2d/3d`, `Matrix3d`, `Plane` and pure helpers |
| `Colors` | `Color`, `EntityColor`, `ColorMethod`, `aciToRgb` |

It differs from .NET where TypeScript has better answers: ids and handles are branded
strings, every data read is `Promise`-returning (the wire is async), and Geometry
values are readonly records, not mutable classes.

## What is a capability, and what is just a function

The descriptor (`crates/cad-ui-core/api/plugin-api.json`, mirrored by
`API_OPERATIONS`) lists **host-served operations** only. The pure helpers — the
Geometry/Colors value constructors, `parsePluginManifest`, `definePlugin` — never
cross the wire, so they have **no row** and are always available. Advertising them
would claim host state that does not exist.

## Lifecycle

1. The host calls `parsePluginManifest(text)` on the manifest **text**, never a path:
   the parser is the only place that decides what a valid manifest is.
2. The host imports the module named by `entry` and calls its default export, which
   the plugin wrapped in `definePlugin` (identity plus shape validation).
3. The host calls `activate(ctx)` once — `ctx.api` is the whole API surface, `ctx.host`
   the snapshot it was built from, `ctx.pluginId` the manifest id — and
   `deactivate(ctx)` once on unload.
4. `createCadApi(bridge)` is the host-side assembly step; it fails closed on an
   `apiVersion` mismatch (`api.host-outdated: <version>`), a malformed snapshot, or a
   descriptor that serves document operations while reporting `documentOwner: 'none'`
   (`api.descriptor_invalid: documentOwner`).

## Capability and refusal model

Before any request the client resolves the operation against the **host's** table:

| State | Meaning | Result |
| --- | --- | --- |
| `supported` | the carrier serves it | the call is made |
| `pending` | named in the contract, not served yet | `api.pending: <Namespace>.<operation>` |
| `unsupported` | this carrier never serves it | `unsupported-runtime: cad_api.<Namespace>.<operation>` |
| *absent* | the host's descriptor does not name it | `api.descriptor_missing: <Namespace>.<operation>` |

A refusal never reaches the host. A `null`/`undefined` answer to a wire call is
`api.invalid_argument: bridge.answer`, and so is a void call that does not answer
`{ accepted: true }` — no placeholders, no empty selections, no default layers, no
silent success. Host failures arrive as `CadApiError`; branch on `code`, not message.

## v1 status

**The descriptor is 21 rows: three `supported`, 18 `pending`.** The served
operations are reflected reads over `cad_api_call`:
`documentManager.mdiActiveDocument` (the open `Document`, or `null`),
`documentManager.documents` (0 or 1 `Document`, `[]` when none is open) and
`database.revision` (`{ database: id }` -> the current revision number). Everything
else is still a reservation — no write path, no prompts, no command dispatch.

| Descriptor rows | v1 |
| --- | --- |
| `ApplicationServices.documentManager.mdiActiveDocument`, `.documents` (2) | `supported` — reflected document reads |
| `ApplicationServices.document.lock` (1) | `pending` — no lock path yet |
| `DatabaseServices.database.revision` (1) | `supported` — current revision, read every call |
| `DatabaseServices` (7 more) | `pending` — no object read/write or symbol-table path yet |
| `EditorInput` (9) | `pending` — no interactive prompt path yet |
| `Runtime.runtime.registerCommand` (1) | `pending` — no chrome-local admission path yet |

`DocumentManager.mdiActiveDocument()` and `documents()` are **methods returning
promises**, not AutoCAD's properties: the value crosses an asynchronous wire, and a
synchronous accessor could only be honest by serving a stale cache. A host with no
snapshot yet refuses with `cad_waiting`, and that rejection reaches the plugin as-is.
`Database.revision()` is likewise read from the host on every call — it is the value
`expectedRevision` is compared against, so it is never cached.

## Transactions

`startTransaction({ expectedRevision })` is synchronous: the host is not asked to
open a transaction, so edits buffer client-side. `expectedRevision` is **required**
for the outermost transaction — an omitted precondition would be silent
last-writer-wins, so the caller reads `database.revision()` first. Only the outermost
`commit()` crosses the wire, as one request carrying
`{ document, transactionId, expectedRevision, edits }`; it resolves
`{ applied: true, revision, created }` with the host's new revision and the ids it
actually created.

- Nesting is strictly LIFO for `commit`/`abort` (`api.transaction.nesting_order`), at
  most 8 deep (`api.transaction.nesting_limit`), and a transaction closes once
  (`api.transaction.closed`). Reads and buffered writes are allowed at any depth.
- A nested commit is a savepoint release: it merges into the parent, applies nothing
  and reports `{ applied: false, revision: null, created: [] }` — no revision is
  invented. A nested abort discards its own buffer.
- `abort()` is **local only** in v1: the host never minted a transaction handle, so
  nothing remote is rolled back. A future `transaction.start` that mints a host id
  turns abort into a wire call.
- `expectedRevision` is optimistic concurrency: a stale value is rejected by the host
  as `api.revision-conflict` (`details.actualRevision` when the host supplies one).
- Creates are a closed set (`Line`/`Circle`/`Arc`, a layer **name** and the active
  space); one transaction may not edit one entity twice
  (`api.invalid_argument: edits.duplicate`).

## Identity

`ObjectId` and `Handle` are branded opaque strings. `DBObject.handle` is the imported
`source_handle` provenance only and is `null` for entities created through this API —
it is never synthesized. Layers and blocks are **keyed by name** with no handle at
all: `cad-db` layers are `{ id, name, visible }`, and linetype is a symbolic concept,
so inventing handles would create a second identity system.

## Trust model

Plugins are untrusted **same-privilege ES modules**, not a sandbox: they run in the
host's realm, reach any global it exposes, and keep running after `deactivate` if they
schedule work themselves. The capability table limits what this package asks the host
to do; it does not contain a malicious plugin. Isolation, if needed, is a process or
realm decision for the host.

`npm test` runs `tsc --noEmit` plus the behavioural suites (Node 24 type stripping),
including the cross-language contract check against
`crates/cad-ui-core/api/plugin-api.json`. Passing them is contract evidence for this
package only — not platform, GPU or end-user acceptance evidence for any host.
