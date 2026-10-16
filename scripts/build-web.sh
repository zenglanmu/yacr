#!/usr/bin/env bash
# build-web.sh — RETIRED: the browser UI is NOT IMPLEMENTED.
#
# The Slint/wasm browser host was retired: the web entry points in `apps/app-web`
# only reject with a diagnostic, and the JS shell under `apps/app-web/web/` has
# been removed. There is no web bundle to build, so this entry point fails loudly
# instead of emitting a stub `web-dist/` that could be deployed. Re-enable only
# together with a replacement browser UI and a new, verified build/publish
# pipeline.
set -euo pipefail
echo "build-web.sh: NOT IMPLEMENTED — the browser UI was retired; there is no web bundle to build." >&2
exit 1
