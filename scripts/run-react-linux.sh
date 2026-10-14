#!/usr/bin/env bash
# Launch the React native Linux host (Tauri + native Rust CAD) from any directory.
#
# This entry point:
#   1. requires a Linux display (X11/XWayland) and forces GDK_BACKEND=x11 — the
#      host refuses other backends (see docs/adr/0006-react-native-linux-experiment.md),
#   2. builds the designer UI (`apps/app-react-linux/ui`) with Vite: `npm ci`
#      when `node_modules` is missing, then `npm run build`,
#   3. runs the host with `custom-protocol` so the built `ui/dist` is embedded.
#
# It NEVER installs system packages, NEVER unlocks the session and NEVER takes a
# screenshot. Required system dev packages (WebKitGTK 4.1, GTK3, Node/npm) and
# the authorized portal helper are documented in docs/react-linux-app.md.
set -euo pipefail

root="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
cd "$root"

if [[ -z "${DISPLAY:-}" ]]; then
    echo "error: DISPLAY is not set; a Linux display (X11/XWayland) is required." >&2
    echo "       This host does not run headless/over SSH without a display." >&2
    exit 2
fi

# The host rejects any other backend; force x11 explicitly (no silent fallback).
export GDK_BACKEND=x11

ui_dir="$root/apps/app-react-linux/ui"
if [[ ! -d "$ui_dir/node_modules" ]]; then
    echo "[run] node_modules missing; running npm ci in $ui_dir"
    (cd "$ui_dir" && npm ci)
fi
echo "[run] building UI (vite build)"
(cd "$ui_dir" && npm run build)

echo "[run] starting yacr-react-probe (custom-protocol)"
exec cargo run --release --locked -p app-react-linux --bin yacr-react-probe \
    --features tauri/custom-protocol -- "$@"
