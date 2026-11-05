#!/usr/bin/env bash
# Build the standalone vendor plugin (TypeScript) and publish it to the app's
# public directory. The plugin is intentionally outside every app package so it
# can be developed and versioned independently (a git submodule at plugins/tch).
#
# The sources are split into modules (`bit-reader.ts`, `wall.ts`, `column.ts`,
# `opening.ts`, `text.ts`); rolldown bundles them into the single ES module the WebView
# loads. Do not replace this with per-file emits: the host fetches one
# `plugins/tch.js` only.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$root/plugins/tch/src/index.ts"
if [[ ! -f "$src" ]]; then
  echo "vendor plugin source missing at $src" >&2
  echo "initialize the submodule: git submodule update --init --recursive" >&2
  exit 1
fi
rolldown_bin="$root/apps/desktop-ui/node_modules/.bin/rolldown"
if [[ ! -x "$rolldown_bin" ]]; then
  echo "vendor plugin build requires rolldown at $rolldown_bin" >&2
  exit 1
fi

public_dir="$root/apps/desktop-ui/public/plugins"
out="$root/plugins/tch/dist/index.js"
mkdir -p "$(dirname "$out")"
rm -f "$public_dir/tch.js"
"$rolldown_bin" "$src" \
  --format esm --platform browser --file "$out"
[[ -s "$out" ]] || { echo "vendor plugin bundle is missing or empty: $out" >&2; exit 1; }
mkdir -p "$public_dir"
cp "$out" "$public_dir/tch.js"
