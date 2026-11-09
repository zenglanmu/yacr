#!/usr/bin/env bash
# finalize-windows-release.sh -- turn a staged Windows release directory into
# the distributable zip + sha256 sidecar.
#
# `scripts/package-windows-release.sh` builds and stages the release tree; this
# script is the single place that writes `yacr-<version>-windows-x86_64.zip` and
# its `.sha256`. CI signs the staged executables itself with the repository
# SELF-SIGNED PFX (no SignPath, no public CA) and then calls this finalizer, so
# the local UNSIGNED path and the CI SIGNED path cannot drift apart.
#
# It also fills the `Code signing` section of `PACKAGE.txt`, which the package
# script stages with the `@@YACR_SIGNING_NOTE@@` placeholder: an archive that
# claims a signature it does not carry (or vice versa) is refused.
#
# Usage:
#   scripts/finalize-windows-release.sh --signed   STAGE DIST
#   scripts/finalize-windows-release.sh --unsigned STAGE DIST
#
#   STAGE  staged release directory, e.g.
#          target/x86_64-pc-windows-msvc/release/dist/yacr-0.1.2-windows-x86_64
#   DIST   output directory for <name>.zip and <name>.zip.sha256
#
# No build logic lives here: the executables and the staged tree are inputs and
# every missing or already-finalized input is a hard failure.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

PLACEHOLDER='@@YACR_SIGNING_NOTE@@'
SIGNED_NOTE='The executables in the package root carry a SELF-SIGNED Authenticode signature from a repository-issued certificate -- NOT a public CA and not an EV/OV certificate. Windows shows an "unknown publisher" warning unless yacr-dev-codesign.cer (shipped next to the executables) is imported into Trusted Root Certification Authorities (or Trusted Publishers); importing it is a manual, per-machine trust decision. Verify with signtool or Get-AuthenticodeSignature (Status must be Valid and the signer thumbprint must match the shipped certificate). The ZIP container itself is not Authenticode-signed.'
UNSIGNED_NOTE='This archive is UNSIGNED -- it is NOT code-signed. Windows SmartScreen may show an "unknown publisher" warning on first launch.'

case "${1:-}" in
  --signed)   MODE=signed ;;
  --unsigned) MODE=unsigned ;;
  *)
    die "usage: $(basename "$0") --signed|--unsigned STAGE DIST"
    ;;
esac
STAGE="${2:-}"
DIST="${3:-}"
[ -n "$STAGE" ] || die "missing STAGE directory argument"
[ -n "$DIST" ] || die "missing DIST directory argument"
[ -d "$STAGE" ] || die "stage directory does not exist: $STAGE"
[ -f "$STAGE/yacr-windows.exe" ] \
  || die "refusing to package: $STAGE/yacr-windows.exe is missing"
[ -f "$STAGE/PACKAGE.txt" ] || die "refusing to package: $STAGE/PACKAGE.txt is missing"
grep -qF "$PLACEHOLDER" "$STAGE/PACKAGE.txt" \
  || die "$STAGE/PACKAGE.txt has no $PLACEHOLDER placeholder (already finalized?)"

if [ -z "${PYTHON:-}" ]; then
  for candidate in python3 python; do
    if command -v "$candidate" >/dev/null 2>&1; then
      PYTHON="$candidate"
      break
    fi
  done
fi
[ -n "${PYTHON:-}" ] || die "python3 (or python) is required to write the zip"

NAME="$(basename "$STAGE")"
ZIP="${DIST}/${NAME}.zip"
SHA="${ZIP}.sha256"
mkdir -p "$DIST"
rm -f "$ZIP" "$SHA"

"$PYTHON" - "$STAGE" "$MODE" "$PLACEHOLDER" "$SIGNED_NOTE" "$UNSIGNED_NOTE" <<'PY'
import pathlib
import sys

stage, mode, placeholder, signed_note, unsigned_note = sys.argv[1:6]
path = pathlib.Path(stage) / "PACKAGE.txt"
text = path.read_text(encoding="utf-8")
if placeholder not in text:
    raise SystemExit(f"{path}: no {placeholder} placeholder")
note = signed_note if mode == "signed" else unsigned_note
path.write_text(text.replace(placeholder, note), encoding="utf-8")
if placeholder in path.read_text(encoding="utf-8"):
    raise SystemExit(f"{path}: placeholder survived the rewrite")
PY

"$PYTHON" - "$STAGE" "$ZIP" <<'PY'
import hashlib
import pathlib
import sys
import zipfile

stage = pathlib.Path(sys.argv[1]).resolve()
out = pathlib.Path(sys.argv[2])
if not (stage / "yacr-windows.exe").is_file():
    raise SystemExit(f"refusing to zip: {stage}/yacr-windows.exe is missing")
# Entries keep the top-level `<name>/...` prefix (the archive extracts into one
# directory), exactly like the previous in-package-script implementation.
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
    for path in sorted(stage.rglob("*")):
        archive.write(path, path.relative_to(stage.parent).as_posix())
digest = hashlib.sha256(out.read_bytes()).hexdigest()
(out.parent / f"{out.name}.sha256").write_text(
    f"{digest}  {out.name}\n", encoding="utf-8"
)
PY

[ -s "$ZIP" ] || die "no zip was produced at $ZIP"
[ -s "$SHA" ] || die "no checksum was produced at $SHA"
"$PYTHON" - "$ZIP" "$SHA" <<'PY'
import hashlib
import pathlib
import re
import sys

zip_path, sha_path = (pathlib.Path(arg) for arg in sys.argv[1:3])
expected = sha_path.read_text(encoding="utf-8").split()[0]
if not re.fullmatch(r"[0-9a-f]{64}", expected):
    raise SystemExit(f"{sha_path}: not a sha256 digest: {expected!r}")
actual = hashlib.sha256(zip_path.read_bytes()).hexdigest()
if actual != expected:
    raise SystemExit(f"{sha_path}: digest does not match {zip_path.name}")
PY

echo
echo "artifact : ${ZIP}"
echo "size     : $(wc -c < "$ZIP") bytes"
echo "sha256   : $(cut -d' ' -f1 "$SHA")"
if [ "$MODE" = "signed" ]; then
  echo "signing  : SELF-SIGNED Authenticode (*.exe; repository cert, NOT a public CA)"
else
  echo "signing  : UNSIGNED"
fi
