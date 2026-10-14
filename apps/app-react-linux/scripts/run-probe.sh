#!/usr/bin/env bash
# Run the React native Linux probe host with the required X11 backend.
#
# The probe refuses to start unless GDK_BACKEND is x11 (see
# docs/adr/0006-react-native-linux-experiment.md), so this entry point sets it
# explicitly. The frontend is served by the Vite dev server at
# http://localhost:1420 (tauri.conf.json `build.devUrl`); for a production
# bundle, build the designer's `ui/dist` and compile with `custom-protocol`.
set -euo pipefail

root="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
cd "$root"

export GDK_BACKEND=x11
exec cargo run --locked -p app-react-linux --bin yacr-react-probe -- "$@"
