# `apps/app-android` — Tauri 2 Android host (`native-below`)

A **`native-below` carrier** (ADR 0027): the shared React chrome
(`apps/desktop-ui`, built with `YACR_RUNTIME=android` into `dist/`) runs in the
Tauri WebView, while the **native Rust `CadSession` owns `cad-db`** and
`cad-render-wgpu` presents with Vulkan to a `SurfaceView` layered **beneath** the
transparent WebView. The wasm `dom-canvas` session is never instantiated; the
native shell never creates a second document authority (`AGENTS.md` iron rule 8).

Document capabilities are advertised from `src-tauri/src/policy.rs`
(`CAN_OPEN`/`CAN_SAVE = true`, `CAN_NEW`/`CAN_PLOT = false`,
`IMPORT_WORKER = false`):

- **Open** — SAF `ACTION_OPEN_DOCUMENT` (`CATEGORY_OPENABLE`, `*/*`, read +
  persistable grant). A Kotlin worker reads the bytes; Rust polls a single-slot
  snapshot and delivers `FileEvent::Opened` through the shared `PlatformFiles`
  funnel. The wildcard MIME is deliberate: `.dwg` providers often report
  `application/octet-stream`.
- **Save** — SAF `ACTION_CREATE_DOCUMENT` (`application/octet-stream`,
  `EXTRA_TITLE`) followed by a **DXF export copy** through
  `ContentResolver.openOutputStream(uri, "wt")`. DWG save stays an explicit
  deferred refusal (`save.dwg_deferred`), and overwriting the opened source URI
  is refused. New and Plot remain unsupported.
- **External "Open with"** — `.dwg` VIEW filters come from
  `bundle.fileAssociations` (tauri-build injects them into the committed
  manifest); `MainActivity` handles cold (`onCreate`) and warm (`onNewIntent`)
  intents through the same read slot.
- **Limits** — a `content://` target has no filesystem directory, so sibling
  xrefs/images are unavailable; import is synchronous on the tick thread, so a
  very large DWG can stall rendering.

Modules:

- `src-tauri/src/host.rs`, `host/{tick,pump}.rs` — Tauri entry, shared command
  registry, `MainEventsCleared` tick and redraw wake-up.
- `src-tauri/src/surface.rs` — frozen Kotlin→Rust JNI surface callbacks and the
  Vulkan-only presenter (`yacr: unsupported-gpu: software-vulkan` on a software
  adapter, never a GLES/CPU fallback).
- `src-tauri/src/saf.rs` — Android-only SAF JNI bridge for the five
  `document*` instance methods; not compiled on the host target, where the
  `AndroidFiles` entry points refuse explicitly instead.
- `src-tauri/src/platform.rs` — `AndroidFiles` (SAF open/save contract),
  `AndroidConfigStore` (`filesDir` config/preference store) and surface/resources.
- `src-tauri/src/policy.rs`, `input.rs`, `refusal.rs`, `report.rs`, `contract.rs`
  — pure policy, canvas scale / coordinate mapping, stable markers, report lines
  and source-structure contracts (all host-testable).
- `src-tauri/gen/android/app/src/main/java/io/github/zenglanmu/yacr/MainActivity.kt` — the
  hand-written, committed Activity (SurfaceView underneath the WebView, insets,
  asset extraction, `document*` delegators).
- `src-tauri/gen/android/app/src/main/java/io/github/zenglanmu/yacr/DocumentPicker.kt` — the
  committed SAF picker/read slot; it holds at most one interaction result and
  never a document.
- `src-tauri/gen/android/app/proguard-rules.pro` — R8 keep rule for the five
  `document*` methods (without it the release APK loses `documentLaunchOpen` from
  `classes.dex`).
- Contract, layout, build, **evidence and NOT VERIFIED boundaries**:
  `docs/android-host.md`
- Build: `bash scripts/build-android.sh --release --target aarch64` (needs
  `ANDROID_HOME`, `NDK_HOME`, `JAVA_HOME` = JDK 17)
- UI bundle (also the Tauri `beforeBuildCommand`): `bash scripts/build-android-ui.sh`
- Static contract gate: `python3 scripts/test-android-host.py`
- `src-tauri/gen/android` is the committed Gradle project (`tauri android init`);
  build outputs, `local.properties`, `jniLibs/*.so` and `gen/schemas` are ignored.
  The manifest's `tauri-file-associations` block is AUTO-GENERATED from
  `tauri.conf.json`; edit the config, not the manifest.
