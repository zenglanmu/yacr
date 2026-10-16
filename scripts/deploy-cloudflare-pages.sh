#!/usr/bin/env bash
# deploy-cloudflare-pages.sh — RETIRED: the browser UI is NOT IMPLEMENTED.
#
# The web bundle this script published no longer exists: the browser host was
# retired and `scripts/build-web.sh` now fails. Publishing is disabled so the
# Cloudflare/CI path can never ship a stub bundle. Re-enable only together with a
# replacement browser UI and a new, verified publish pipeline.
set -euo pipefail
echo "deploy-cloudflare-pages.sh: NOT IMPLEMENTED — the browser UI was retired; nothing is published." >&2
exit 1
