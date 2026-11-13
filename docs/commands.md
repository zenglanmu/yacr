# IPC command reference

The typed read commands shared by every carrier. Names, argument shapes and reply
types come from the single registry
(`crates/cad-ui-core/src/session/registry.rs`); this file documents the **JSON
payloads** a UI codes against. A command that is not in the registry does not
exist, and a refusal is always an explicit `Err(String)` — never an empty
success.

Both carriers serve the reads from their own authority:

| carrier | `settings` / `layers` / `fonts` |
| --- | --- |
| desktop (`native-below`: Linux Wayland, Windows, macOS, Android) | the owner tick mirrors the main-thread `CadSession` into `Shared`; the IPC thread never touches the `!Send` session |
| `dom-canvas` (Linux X11/XWayland, Web) | the wasm `CadSession` answers directly in the same thread |

Before the first snapshot a mirror-backed read answers the stable reason string
`cad_waiting` (a loading state, not an error and not an empty registry).

## `cad_layers`

The layer-panel projection: the drawing's own layer table plus the session-scoped
temporary overrides. No document is an explicit empty answer
(`{"layers":[],"active":null,"overridden":0}`), never a synthetic `0` row.

```json
{
  "layers": [
    {
      "name": "0",
      "visible": true,
      "databaseVisible": true,
      "overridden": false,
      "current": true,
      "color": { "aci": 7, "rgb": "#ffffff" },
      "linetype": "Continuous",
      "lineweight": "ByLayer",
      "transparency": "0",
      "locked": false,
      "frozen": false,
      "off": false,
      "plottable": true
    }
  ],
  "active": "0",
  "overridden": 0
}
```

- `color.aci` is `null` for a true-colour layer; `rgb` is always a lowercase
  `#rrggbb` display value.
- `lineweight` is the write grammar's own unit: `"ByLayer"`, `"Default"` or the
  DXF `370` integer in 1/100 mm (`"35"` = 0.35 mm).
- `transparency` is **percent transparent** (`"0"` = opaque).
- Only `off`/`frozen` feed `visible`; the other stored attributes are display
  values (see `docs/panels.md`).

Mutations travel on typed write commands (`cad_set_layer_visible`,
`cad_restore_layers`, `cad_set_active_layer`, `cad_create_layer`,
`cad_rename_layer`, `cad_delete_layer`, `cad_set_layer_property`); their ack
means **queued**, and the panel re-reads `cad_layers` after the owner tick.

## `cad_fonts`

The read-only font report: every face the host's last shaping-font load touched,
projected one-to-one from the real `cad_platform::fonts::FontLoadReport`. The
host never registers, retries or invents a face. A drawing that needed no fonts
answers `available: true` with `faces: []`; a host that cannot report the
registry at all answers `available: false` with a named machine `reason`.

```json
{
  "available": true,
  "reason": null,
  "fontsDir": "/opt/yacr/fonts",
  "catalogEntries": 4,
  "locale": "zh-Hans",
  "defaultFace": "NotoSansCJKsc-Regular.otf",
  "localeMissing": "zh-Hans (simsun, gbcbig)",
  "requested": ["romans"],
  "textNeeded": true,
  "faces": [
    {
      "key": "romans",
      "file": "romans.shx",
      "kind": "shx",
      "source": "package",
      "status": "loaded",
      "detail": null
    },
    {
      "key": "HZTXT",
      "file": null,
      "kind": "system",
      "source": "system",
      "status": "unresolved",
      "detail": "missing glyph"
    }
  ]
}
```

| field | meaning |
| --- | --- |
| `available` | `false` only when this host has no font registry to report |
| `reason` | named machine key when `available` is `false` (e.g. `host.fonts_unavailable`), else `null` |
| `fontsDir` | the effective `--fonts-dir`, or `null` when unset |
| `catalogEntries` | entries in the fetched font catalog |
| `locale` | the text locale the fallback chain was built for: `latin`, `zh-Hans`, `zh-Hant`, `ja` or `ko` |
| `defaultFace` | the last-resort outline face actually registered, if any |
| `localeMissing` | machine detail (locale + candidates tried) when the locale's preferred face is unavailable; `null` when satisfied or the drawing needs no text |
| `requested` | font keys the current drawing references, read from the drawing even before any package is installed (the Web lazy loader uses this to decide what to fetch) |
| `textNeeded` | whether the current drawing has text even when `requested` is empty (the lazy loader must fetch the default face in this case) |
| `faces` | every touched face, in the load's discovery order |

`faces[].source` is one of `package`, `explicit`, `system`; `faces[].status` is
one of `loaded`, `failed`, `unresolved`, `unsupported`. Both are **stable
lowercase machine labels**, never localized prose (the UI localizes by label).
`file` is the resolved file name and `detail` is the host's verbatim failure
reason when the status is `failed`.

`cad_fonts` is a report, not an action: there is no font-write command. A face
that failed stays visible in `faces` with its real status; nothing is silently
dropped, and nothing is retried on read.

## `cad_api_call`

Performs **one** plugin-API operation against the host's reflected snapshot. It is
the *doing* half of the surface; `cad_api_status` (below) is the *advertising*
half, and the operation must appear there as `supported` before a client sends it.

```json
{ "namespace": "ApplicationServices", "operation": "documentManager.documents", "parameters": null }
```

The reply envelope carries `ok` plus **exactly one** of `value` / `error` (the
absent half is omitted, never `null`):

```json
{ "ok": true, "value": { "documents": [] } }
{ "ok": false, "error": "unsupported-runtime: cad_api.EditorInput.editor.getPoint" }
```

The served operations are the `supported` rows of the descriptor:

| namespace.operation | `value` |
| --- | --- |
| `ApplicationServices.documentManager.mdiActiveDocument` | `{ "document": <descriptor> }` or `{ "document": null }` |
| `ApplicationServices.documentManager.documents` | `{ "documents": [ <descriptor> ] }` — 0 or 1 |
| `DatabaseServices.database.revision` (`parameters: { "database": "<id>" }`) | `{ "revision": <number> }` |

`<descriptor>` is `{ "id", "name", "isReadOnly", "database": { "id", "version" } }`.

Refusals are explicit and checked in this order:

- any other namespace/operation → `unsupported-runtime: cad_api.<namespace>.<operation>`
  (checked first, before the snapshot and the arguments);
- no reflected snapshot yet (the desktop owner-tick mirror has not been populated)
  → `cad_waiting`, the shared "ask again after a tick" word;
- `database.revision` whose `parameters.database` is missing, not a string, or not
  the snapshot's database id → `api.invalid_argument: database`;
- `database.revision` while the snapshot exists but holds no document →
  `cad_api.no_document`.

`{ "document": null }` and `{ "documents": [] }` are real answers (no document is
open), not waits and not failures. Every answer is a **derived** projection of the
host controller's snapshot: `cad-db` remains the only authority, no transaction is
opened, and the mirror is never written back. On `native-below` the snapshot lags
the owner tick by at most one tick; `dom-canvas` reads the live session.

## `cad_api_status`

The plugin / secondary-development API **capability descriptor**: which
**host-served** operations this build offers, and which it merely reserves. It
answers one pure value and touches no document, no session state and no host
resource.

```json
{
  "apiVersion": 1,
  "namespaces": ["ApplicationServices", "DatabaseServices", "EditorInput", "Runtime", "Geometry", "Colors"],
  "operations": [
    {
      "namespace": "DatabaseServices",
      "operation": "transaction.commit",
      "status": "pending",
      "reason": "api.pending: DatabaseServices.transaction.commit"
    },
    {
      "namespace": "EditorInput",
      "operation": "editor.getPoint",
      "status": "pending",
      "reason": "api.pending: EditorInput.editor.getPoint"
    },
    {
      "namespace": "Runtime",
      "operation": "runtime.registerCommand",
      "status": "pending",
      "reason": "api.pending: Runtime.runtime.registerCommand"
    }
  ]
}
```

The full row list is `crates/cad-ui-core/api/plugin-api.json` — the one authority
this command parses; do not transcribe it into a client. It has **21 rows: 18
`pending` + 3 `supported`** (`documentManager.mdiActiveDocument`,
`documentManager.documents`, `database.revision`). A row exists only for an
operation the host answers over the wire; the pure value helpers (`Geometry`,
`Colors`) and the manifest parser are package features in
`packages/cad-plugin-api`, not host capabilities, so they have no row and are
available regardless of the table.

`status` is one of three stable lowercase labels:

| status | meaning | `reason` |
| --- | --- | --- |
| `supported` | implemented in this build | omitted entirely |
| `pending` | reserved by the contract, not implemented in this build | required, always `api.pending: <Namespace>.<operation>` |
| `unsupported` | deliberately not offered | required, a named marker |

A `supported` row has no `reason` key at all (not `null`), and a `pending` or
`unsupported` row always carries one — a capability is never advertised as an
empty success.

**In this build 18 of the 21 rows are `pending`** — no other host operation is
served yet, so a client refuses those by name (`api.pending:
<Namespace>.<operation>`). The three `supported` rows are served by `cad_api_call`
(above); pure package features (Geometry, Colors, manifest parsing) never consult
this table at all. The command advertises the contract; it does not itself read or
write the document.

`cad_api_status` is a `shared` command: `native-below` and `dom-canvas` carriers
serve the identical answer from the same registry row. It is **not** a CAD
command — it is absent from `cad_commands` and cannot be typed as a selector.

## `cad_plugin_*` (vendor plugin registry, native shell only)

These five commands belong to the **shell**, not to a CAD document. They are not
`COMMAND_TABLE` selectors, so `scripts/check-command-docs.py` does not admit
them as CAD command names; the contract below and
`docs/vendor-plugin.md` §8 are their documentation. `host/invoke.ts` lists them
as shell-only. A carrier without the registry refuses them by name
(`unsupported-runtime: cad_plugin_<name>`, Web) or answers a `supported: false`
frame with `plugin.unsupported_carrier: dom-canvas` (Linux X11/XWayland).

All five serve the same `native-below` registry. One decoder is active at a
time; changes persist immediately but take effect on the next launch
(`restartRequired = activeDecoder != sessionActiveDecoder`).

| command | arguments | reply |
| --- | --- | --- |
| `cad_plugin_status` | — | `PluginStatusFrame` |
| `cad_plugin_install` | `{ filename: string, payload: string (base64), meta?: { id?, name?, version? } }` | `PluginMutationResult` |
| `cad_plugin_enable` | `{ id: string, enabled: boolean }` | `PluginMutationResult` |
| `cad_plugin_uninstall` | `{ id: string }` | `PluginMutationResult` |
| `cad_plugin_read_entry` | `{ id: string }` | `{ payload: string (base64), entry: "plugin.js" }`; the built-in id is refused |

`PluginStatusFrame`:

```json
{
  "supported": true,
  "carrier": "native-below",
  "plugins": [
    {
      "id": "tch",
      "name": "Tianzheng TCH",
      "version": "0.1.0",
      "abi": 1,
      "source": "builtin",
      "enabled": true,
      "sessionActive": true,
      "status": "ok"
    }
  ],
  "activeDecoder": "tch",
  "sessionActiveDecoder": "tch",
  "restartRequired": false,
  "diagnostics": []
}
```

`PluginMutationResult` carries the registry snapshot after the attempt, so a
refusal is never an empty success:

```json
{
  "accepted": false,
  "reason": "plugin.unsupported_carrier: dom-canvas",
  "restartRequired": false,
  "status": {
    "supported": false,
    "carrier": "dom-canvas",
    "plugins": [
      {
        "id": "tch",
        "name": "Tianzheng TCH",
        "version": "0.1.0",
        "abi": 1,
        "source": "builtin",
        "enabled": true,
        "sessionActive": true,
        "status": "ok"
      }
    ],
    "activeDecoder": "tch",
    "sessionActiveDecoder": "tch",
    "restartRequired": false,
    "diagnostics": ["plugin.unsupported_carrier: dom-canvas"]
  }
}
```

- `payload`/`filename` install path: the id is the module-provided `id` when it
  is valid, otherwise the basename derived slug (`.js`/`.mjs`). The reserved id
  `tch` and an already-installed id are refused (`plugin.reserved_id` /
  `plugin.already_installed`), and the entry is capped at 4 MiB. `name`/`version`
  default to the id and `0.0.0`.
- `cad_plugin_enable` is radio semantics: `enabled: true` makes `id` the active
  decoder; `enabled: false` clears it only when it is active (disabling a
  non-active row is an accepted no-op).
- `cad_plugin_uninstall` deletes the plugin directory; uninstalling the active
  decoder also writes `activeDecoder: null`. The built-in `tch` cannot be
  uninstalled.
- `cad_plugin_read_entry` returns the installed bundle bytes for the vendor
  loader; the built-in refuses because its bundle comes from the application's
  own URL.
- A malformed policy is reported (`plugin.policy_invalid`) and status falls back
  to the built-in default, but mutations refuse rather than overwrite it.

**Evidence scope**: this is the implemented `native-below` contract
(Linux contract tests plus shared-layer static wiring). Windows/macOS/Android
runtime, device, real GUI/browser, GPU and pixel behaviour are
**NOT VERIFIED**.
