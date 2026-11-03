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
