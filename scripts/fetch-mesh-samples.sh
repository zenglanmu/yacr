#!/usr/bin/env bash
# Fetch a small, reproducible third-party mesh corpus for the P-MESH-1a/1b
# tests. The samples are **not** committed: they are downloaded into a
# directory outside the repository (default /tmp/opencode/yacr-mesh-samples).
#
# Usage:
#   bash scripts/fetch-mesh-samples.sh [<directory>]
#   YACR_TEST_MESH_DIR=<directory> cargo test -p cad-cli-tools \
#     --test mesh_real_samples --locked -- --nocapture
#
# Sources and licences (checked 2026-11-26):
#   * `manifold` upstream test models, Apache-2.0 repository
#     <https://github.com/elalish/manifold>, pinned commit below. The upstream
#     repository is the authority for each model's provenance.
#   * Stanford 3D Scanning Repository bunny (research use; not redistributed
#     here) <https://graphics.stanford.edu/data/3Dscanrep/>.
#
# The SHA-256 list is the evidence that the corpus used for a run is the corpus
# this script names. A mismatch fails the script; it never silently accepts a
# different file.

set -euo pipefail

dir="${1:-${YACR_MESH_SAMPLE_DIR:-/tmp/opencode/yacr-mesh-samples}}"
mkdir -p "$dir"

MANIFOLD_SHA="c5ffc47388b86250c2eb70c2ddf0b64d3678207c"
BUNNY_URL="https://graphics.stanford.edu/pub/3Dscanrep/bunny.tar.gz"

manifest="$(cat <<'EOF'
4276e824ed0e6c5e776357561adfae51b650f961dbfcb3be4c42567191e7a607  Cray_left.obj
064af131332e0713eed0c712d822321d65da281c8c53f67aec6fff7557fba1e6  Cray_right.obj
8faa052bb08cf1625eec97b6508baceee11eec9cf28f83f6a9d4547ab15d7761  bun_zipper_res2.ply
3131e24d27914a363a5ac0e0de9fa4e2dfc25a25d886a67ffa47a6e4ca5b32ee  dedupe_stale_duplicate.obj
4b55c75cd730c6912a31f7ac7fde0535903cec5c9012bf5a7ecbff45f0e3311b  degenerate_split_quad_shared_a.obj
eec8015cd397cec31d8269858cef94a96043722606d83b8b6fea223392009f7f  degenerate_split_quad_shared_b.obj
a7f2cdaa5bcf97f0a6c9e7b8de46b653955e7af4e95bab0f576608881ebf9f26  hull-body.obj
9b40b7fda026c2b08e82b57df2dbc8460e35582720cf44a6b563d594d64ef513  hull-mask.obj
497bd6df29f4c6fd4f3e85f72c31d88e95f01eb11ff1ec51b674e5b459fafcea  openscad-nonmanifold-crash.obj
fd5b5c87b1fc5597243f9e1d73eb46470ebecb0efd9cec966a8de3809d1e48b5  r1.obj
78d82c3295a7022c9c18d6bca54a491443c6db03a5b54f8739a23bffea96c82a  r2.obj
EOF
)"

fetch_manifold() {
  local name="$1"
  local target="$dir/$name"
  if [ -s "$target" ]; then
    echo "keep $name"
    return
  fi
  echo "fetch manifold/$name"
  curl -fsSL --retry 4 --retry-delay 2 --max-time 120 \
    -H 'Accept: application/vnd.github+json' \
    "https://api.github.com/repos/elalish/manifold/contents/test/models/${name}?ref=${MANIFOLD_SHA}" \
    | python3 -c 'import base64,json,sys; sys.stdout.buffer.write(base64.b64decode(json.load(sys.stdin)["content"]))' \
    > "$target"
}

fetch_bunny() {
  local target="$dir/bun_zipper_res2.ply"
  if [ -s "$target" ]; then
    echo "keep bun_zipper_res2.ply"
    return
  fi
  echo "fetch Stanford bunny"
  local work
  work="$(mktemp -d)"
  curl -fsSL --retry 4 --retry-delay 2 --max-time 300 -o "$work/bunny.tar.gz" "$BUNNY_URL"
  tar -xzf "$work/bunny.tar.gz" -C "$work"
  cp "$work/bunny/reconstruction/bun_zipper_res2.ply" "$target"
  rm -rf "$work"
}

for name in \
  Cray_left.obj Cray_right.obj dedupe_stale_duplicate.obj \
  degenerate_split_quad_shared_a.obj degenerate_split_quad_shared_b.obj \
  hull-body.obj hull-mask.obj openscad-nonmanifold-crash.obj r1.obj r2.obj
do
  fetch_manifold "$name"
done
fetch_bunny

echo "verifying SHA-256 in $dir"
( cd "$dir" && printf '%s\n' "$manifest" | sha256sum -c - )
echo "OK: corpus ready at $dir"
echo "run: YACR_TEST_MESH_DIR=$dir cargo test -p cad-cli-tools --test mesh_real_samples --locked -- --nocapture"
