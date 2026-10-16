#!/usr/bin/env bash
# package-linux-release.sh — DISABLED pending replacement release packaging.
#
# The old Flatpak pipeline built the retired Slint headless GUI host and staged
# it with a `--headless` CLI smoke; that path no longer exists. The Linux GUI is
# now the Tauri host (`bin/yacr`), but its release packaging and release
# acceptance have not been re-designed or validated, so this script refuses to
# invent an untested distributable. The Flatpak manifest under `packaging/flatpak`
# is retired with it.
#
# `scripts/run-react-linux.sh` remains the supported *development* entry point
# (and the env-gated evidence mechanism in `apps/app-linux/src/evidence.rs` the
# real-GPU acceptance route). Re-enable only once a replacement package is
# designed and validated end-to-end.
set -euo pipefail
echo "package-linux-release.sh: NOT IMPLEMENTED — Linux release packaging is disabled pending a validated replacement." >&2
exit 1
