#!/usr/bin/env bash
# Run the React native Linux probe host.
#
# The host resolves its own display backend policy before GTK starts
# (`apps/app-linux/src/backend.rs`): an explicit `GDK_BACKEND=x11|wayland` is
# preserved; otherwise it prefers a native Wayland session
# (`WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR`) and falls back to X11/`DISPLAY`. This
# wrapper deliberately does **not** force `GDK_BACKEND`, so the user's explicit
# backend is honoured; the host refuses an unsupported/ambiguous value rather
# than falling back silently.
#
# The frontend is served by the Vite dev server at http://localhost:1420
# (tauri.conf.json `build.devUrl`); for a production bundle, build the
# designer's `ui/dist` and compile with `custom-protocol` (see
# scripts/run-react-linux.sh).
set -euo pipefail

root="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
cd "$root"

exec cargo run --locked -p app-linux --bin yacr -- "$@"
