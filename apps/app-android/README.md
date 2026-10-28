# `apps/app-android` — Tauri 2 Android host

An **asset/WebView carrier**: the shared React chrome (`apps/desktop-ui`, built with
`YACR_RUNTIME=android` into `dist/`) runs in the Android WebView and the **wasm
`dom-canvas` CAD session owns the document and the pixels** (ADR 0024, `AGENTS.md`
iron rule 8). The native side registers **only** the two shell-only reports the
chrome sends (`canvas_session_status`, `canvas_runtime_failed`) and holds no CAD
state.

- Contract, layout, build, **evidence and NOT VERIFIED boundaries**: `docs/android-host.md`
- Build: `bash scripts/build-android.sh --release --target aarch64` (needs
  `ANDROID_HOME`, `NDK_HOME`, `JAVA_HOME` = JDK 17)
- UI bundle (also the Tauri `beforeBuildCommand`): `bash scripts/build-android-ui.sh`
- Static contract gate: `python3 scripts/test-android-host.py`
- `src-tauri/gen/android` is the committed Gradle project (`tauri android init`);
  build outputs, `local.properties`, `jniLibs/*.so` and `gen/schemas` are ignored.
