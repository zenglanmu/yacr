#!/usr/bin/env bash
# package-windows-release.sh — RETIRED: the Windows GUI is NOT IMPLEMENTED.
#
# `apps/app-windows` is a NOT IMPLEMENTED failure stub (bin `yacr-windows`), not
# a GUI. There is nothing to package, so this entry point fails loudly instead of
# emitting a stub release zip that could be published. Re-enable only together
# with a replacement Windows UI and a re-validated packaging pipeline.
set -euo pipefail
echo "package-windows-release.sh: NOT IMPLEMENTED — the Windows GUI was retired; no release is packaged." >&2
exit 1
