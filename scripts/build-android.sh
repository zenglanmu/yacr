#!/usr/bin/env bash
# build-android.sh — RETIRED: the Android UI is NOT IMPLEMENTED.
#
# The Android host was retired to an explicit NOT IMPLEMENTED placeholder
# (`apps/app-android` logs the retirement and aborts). There is no packaging step
# for a stub APK, so this entry point fails loudly rather than producing a signed
# APK that no one could install as a CAD app. Re-enable only together with a
# replacement Android UI. (The `android-check` CI job still compiles the stub as
# a plain `cargo check` gate; that is not this packaging script.)
set -euo pipefail
echo "build-android.sh: NOT IMPLEMENTED — the Android UI was retired; no APK is produced." >&2
exit 1
