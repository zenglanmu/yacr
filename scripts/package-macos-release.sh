#!/usr/bin/env bash
# package-macos-release.sh — RETIRED: the macOS GUI is NOT IMPLEMENTED.
#
# `apps/app-macos` is a NOT IMPLEMENTED failure stub, not a GUI. There is nothing
# to package, so this entry point fails loudly instead of emitting a stub app
# bundle that could be published. Re-enable only together with a replacement
# macOS UI and a re-validated packaging pipeline (run on a real macOS runner).
set -euo pipefail
echo "package-macos-release.sh: NOT IMPLEMENTED — the macOS GUI was retired; no release is packaged." >&2
exit 1
