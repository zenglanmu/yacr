#!/usr/bin/env bash
# Launch the React native Linux host (Tauri + native Rust CAD) from any directory.
#
# This entry point:
#   1. admits a Linux display session — either a native Wayland session
#      (`WAYLAND_DISPLAY` **and** `XDG_RUNTIME_DIR`) or X11/XWayland (`DISPLAY`)
#      — and never forces `GDK_BACKEND`. An explicit `GDK_BACKEND=x11|wayland` is
#      preserved; an unset one is resolved authoritatively by the host (Wayland
#      preferred; see apps/app-linux/src/backend.rs,
#      docs/adr/0006-react-native-linux-experiment.md),
#   2. builds the designer UI (`apps/app-linux/ui`) with Vite: `npm ci`
#      when `node_modules` is missing, then `npm run build`,
#   3. runs the host with `custom-protocol` so the built `ui/dist` is embedded.
#
# It NEVER installs system packages, NEVER unlocks the session and NEVER takes a
# screenshot. Required system dev packages (WebKitGTK 4.1, GTK3, Node/npm) and
# the authorized portal helper are documented in docs/react-linux-app.md.
set -euo pipefail

root="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
cd "$root"

# Coarse admission only: the host (`apps/app-linux/src/backend.rs`) is the single
# source of truth and performs the authoritative `GDK_BACKEND` resolution,
# including the precise exit-2 diagnostic for an unsupported/ambiguous value or
# a selected backend that is unavailable. This guard mirrors the same
# preconditions (native Wayland needs `WAYLAND_DISPLAY` *and* `XDG_RUNTIME_DIR`;
# X11/XWayland needs `DISPLAY`) so a headless invocation fails fast without
# duplicating the full policy. No backend is ever forced here (no silent
# fallback). An explicit `GDK_BACKEND` bypasses this guard and is validated by
# the host (exit 2 when invalid).
if [[ -z "${GDK_BACKEND:-}" ]]; then
    if [[ -n "${WAYLAND_DISPLAY:-}" && -n "${XDG_RUNTIME_DIR:-}" ]]; then
        : # native Wayland session (the host will prefer it)
    elif [[ -n "${DISPLAY:-}" ]]; then
        : # X11/XWayland session
    else
        echo "error: no display session detected; a Linux display is required." >&2
        echo "       Native Wayland needs both WAYLAND_DISPLAY and XDG_RUNTIME_DIR;" >&2
        echo "       X11/XWayland needs DISPLAY. Or choose explicitly with" >&2
        echo "       GDK_BACKEND=x11|wayland (validated by the host, exit 2 if unsupported)." >&2
        echo "       This host does not run headless/over SSH without a display." >&2
        exit 2
    fi
fi

ui_dir="$root/apps/app-linux/ui"
if [[ ! -d "$ui_dir/node_modules" ]]; then
    echo "[run] node_modules missing; running npm ci in $ui_dir"
    (cd "$ui_dir" && npm ci)
fi
echo "[run] building UI (vite build)"
(cd "$ui_dir" && npm run build)

echo "[run] starting yacr (custom-protocol)"
exec cargo run --release --locked -p app-linux --bin yacr \
    --features tauri/custom-protocol -- "$@"
