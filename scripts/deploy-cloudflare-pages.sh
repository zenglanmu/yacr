#!/usr/bin/env bash
# deploy-cloudflare-pages.sh — RETIRED: no browser publish path is IMPLEMENTED.
#
# Kept retired: there is no verified publish contract (no credentials, no target
# project, no smoke run) and CI `web-deploy` is still `if: false`. The local bundle
# this script used to publish is now built by `scripts/build-web.sh` and can be
# uploaded **by hand** from apps/app-web/dist/ once a real publish path is agreed.
set -euo pipefail
echo "deploy-cloudflare-pages.sh: NOT IMPLEMENTED — no publish path is configured; build locally with scripts/build-web.sh and upload apps/app-web/dist/ by hand." >&2
exit 1
