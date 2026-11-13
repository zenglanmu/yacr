#!/usr/bin/env bash
# Run the vendored `rvt` patch's own unit tests (ADR-0035, docs/rvt-patches.md).
#
# `vendor/rvt` is excluded from the workspace, so its tests are not part of
# `cargo test --workspace`: this script runs exactly the patched module's tests
# in their own target dir, with the lock file the upstream crate ships (the
# patched source keeps the upstream `Cargo.toml`, so `--locked` also proves the
# vendored manifest was not edited).
#
# A `cargo test <filter>` that matches nothing still exits 0 ("0 filtered out"),
# so the passing count is asserted: deleting the patch's `mod tests` must fail
# this gate rather than silently pass it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/rvt-patch}"
export CARGO_TARGET_DIR

output="$(cargo test \
  --manifest-path "$ROOT/vendor/rvt/Cargo.toml" \
  --locked --lib ifc::gltf "$@" 2>&1)"
printf '%s\n' "$output"

expected='test result: ok\. 4 passed; 0 failed'
if ! grep -Eq "$expected" <<<"$output"; then
  echo "test-rvt-patch: expected 4 passing ifc::gltf tests (ADR-0035); the patch's" >&2
  echo "tests are missing or the run failed. See docs/rvt-patches.md." >&2
  exit 1
fi
