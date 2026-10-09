# Android font assets (`apps/app-android/assets/fonts/`)

This directory holds the **optional** font package the Android host reads via
the activity `AssetManager` (`asset://fonts/…`; see
`apps/app-android/src/lib.rs::android_fonts` and `docs/fonts.md`).

## They are not committed

The font binaries are **gitignored** (`assets/fonts/*`). They are third-party
files from `mlightcad/cad-data`, and bundling them into this repository mixes
their licence with ours, so the repository only records *how* to obtain them.
**Exception**: the repository-level `fonts/` package (QCAD `osifont.ttf`,
GPL-3 + font exception, see `fonts/SOURCE.md`) is committed; the fetch script
below merges it into this asset directory so a packaged APK always carries the
default outline fallback face.

Only this README is tracked here; when the folder is otherwise empty, `cargo
apk` simply packages no fonts and the app reports “font asset not packaged”.

## Download them

```bash
scripts/fetch-android-fonts.sh
# or a custom set:
FONTS="simplex.shx arial.woff" scripts/fetch-android-fonts.sh
```

The script pulls from the same catalogue the web host uses —
`https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts` (override with
`FONT_BASE_URL=`) — and also fetches `fonts.json`, which the host needs to
resolve a drawing's font names.

## How it is wired

- `apps/app-android/Cargo.toml` sets `assets = "assets"` in
  `[package.metadata.android]`; without it `cargo-apk` ignores this directory.
- After a drawing opens, the host collects the referenced fonts
  (`cad_platform::fonts::requested_fonts`) and calls
  `android_fonts::install_fonts` → `CadView::set_fonts`. The status bar reports
  `字体：目录 N，引用 N，计划 N，注册 N，失败 N`.
- A drawing that references a font outside the pack falls back to a registered
  one (the fallback chain is always set), so text still draws.

## Licensing

Ignoring the binaries here is not a licence review. Before distributing an APK
that contains these fonts, verify their terms for your distribution. The default
project policy (see `docs/fonts.md`) is **not** to bundle fonts.
