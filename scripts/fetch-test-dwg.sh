#!/usr/bin/env bash
# fetch-test-dwg.sh -- download a small curated corpus of real DWG files for
# local testing on machines where raw.githubusercontent.com is unreachable.
#
# These samples are NOT part of this repository, are NOT committed by this
# script, and are NOT authorization or compatibility evidence. They are written
# only to a scratch directory (default: /tmp/opencode/dwg-samples).
#
# Usage:
#   scripts/fetch-test-dwg.sh [output-dir]
#
# Sources (only these two hosts are ever contacted):
#   * cdn.jsdelivr.net  mlightcad/cad-data dataset (5 drawings, preferred)
#   * api.github.com    hakanaktt/acadrust test drawings via the base64
#                       contents API (the raw JSON download_url is ignored)
# raw.githubusercontent.com is never used.
#
# Requirements: bash, curl, sha256sum, wc, sed, base64.
#
# Idempotent: a file is re-downloaded only when it is missing or its recorded
# byte size / sha256 does not match. On a post-download mismatch the file is
# deleted and the script exits non-zero.
set -euo pipefail

OUT_DIR="${1:-/tmp/opencode/dwg-samples}"
TIMEOUT="${FETCH_TIMEOUT:-180}"

JSDELIVR_BASE="https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/data"
GH_API="https://api.github.com"
ACADREPO="hakanaktt/acadrust"

# Recorded expectations: name|bytes|sha256|kind|ref
# The sha256 values are the ones recorded in docs/validation.md for these same
# public samples; they are pinned here so a corrupted/upstream-changed download
# fails loudly instead of being accepted.
SAMPLES=(
  "baseline-sample.dwg|81659|4a8e5195e600ac8f6b4e37869cac812fe54c5359b4b97ca19e27ad57422ddc64|jsdelivr|baseline-sample.dwg"
  "canteen.dwg|2618816|818f54cd3b413ce3ab00a6aa849bc29cd8cc8581a39fc31a723691f40141fdbc|jsdelivr|canteen.dwg"
  "lockers.dwg|2245248|fb82491c63fb4d4b5cb80a534bbfe56ee53c46f201a685d9ce1dabbd4bfb3ee4|jsdelivr|lockers.dwg"
  "map-of-uae.dwg|195040|b154073d6edd9b074d5bca40e7ad34bd8b75e89ca67ffb4cc99e10f4f3eb14bc|jsdelivr|map-of-uae.dwg"
  "patient-chairs.dwg|568736|bba7327d855e0efb665fbd1b5aa280d1a07be7450ea8d9a0ac387d1c4f22f51e|jsdelivr|patient-chairs.dwg"
  "anonymous-names.dwg|13947|ea5b55f7e99d2ad412779ef7f3e71ff3bd7f6c4147f6217d936eed02171291b9|github|tests/anonymous-names.dwg"
  "point_object_id.dwg|13637|9eef9375c77d72881dc202b9fb94b9c414a048dfdfab8374419b44830337c49d|github|tests/datatable/point_object_id.dwg"
)

log() { printf '%s\n' "$*" >&2; }

sha256_of() { sha256sum "$1" | cut -d' ' -f1; }
size_of() { wc -c < "$1" | tr -d ' '; }

verify() {
  local dest="$1" exp_size="$2" exp_sha="$3"
  [ -f "$dest" ] || return 1
  [ "$(size_of "$dest")" = "$exp_size" ] || return 1
  [ "$(sha256_of "$dest")" = "$exp_sha" ] || return 1
  return 0
}

# Download one jsDelivr file. Returns non-zero on any HTTP/transfer failure.
fetch_jsdelivr() {
  local ref="$1" dest="$2" url
  url="$JSDELIVR_BASE/$ref"
  if ! curl -fsSL -m "$TIMEOUT" --retry 2 --retry-delay 2 -o "$dest.part" "$url"; then
    rm -f "$dest.part"
    log "ERROR: download failed: $url"
    return 1
  fi
  mv -f "$dest.part" "$dest"
}

# Download one GitHub contents-API file, decoding the base64 "content" field.
# Returns 2 (skip) when the path is absent (404), non-zero otherwise.
fetch_github_base64() {
  local path="$1" dest="$2" url json code
  url="$GH_API/repos/$ACADREPO/contents/$path"
  json="$dest.json.part"
  if ! curl -fsSL -m "$TIMEOUT" -o "$json" "$url"; then
    code="$(curl -sSL -m "$TIMEOUT" -o /dev/null -w '%{http_code}' "$url" || true)"
    rm -f "$json"
    if [ "$code" = "404" ]; then
      log "WARN: GitHub path not found (HTTP 404), skipping: $ACADREPO/$path"
      return 2
    fi
    log "ERROR: GitHub content download failed (HTTP $code): $url"
    return 1
  fi
  if ! command -v base64 >/dev/null 2>&1; then
    rm -f "$json"
    log "WARN: base64 unavailable; cannot decode $ACADREPO/$path, skipping"
    return 2
  fi
  # The contents API escapes newlines as \n inside a JSON string; strip them
  # before decoding. No jq/python dependency.
  if ! sed -n 's/.*"content"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$json" \
       | tr -d '\n' | sed 's/\\n//g' | base64 -d > "$dest.part"; then
    rm -f "$json" "$dest.part"
    log "ERROR: base64 decode failed for $ACADREPO/$path"
    return 1
  fi
  rm -f "$json"
  mv -f "$dest.part" "$dest"
}

mkdir -p "$OUT_DIR"
log "Output directory: $OUT_DIR"

fail=0
rows=()

for entry in "${SAMPLES[@]}"; do
  IFS='|' read -r name bytes sha kind ref <<< "$entry"
  dest="$OUT_DIR/$name"
  case "$kind" in
    jsdelivr) src="$JSDELIVR_BASE/$ref" ;;
    github)   src="$GH_API/repos/$ACADREPO/contents/$ref" ;;
    *)        log "ERROR: unknown source kind '$kind' for $name"; fail=1; continue ;;
  esac

  status=""
  if verify "$dest" "$bytes" "$sha"; then
    status="cached"
  else
    rm -f "$dest"
    if [ "$kind" = "jsdelivr" ]; then
      if fetch_jsdelivr "$ref" "$dest"; then
        status="downloaded"
      else
        status="failed"
        fail=1
      fi
    else
      if fetch_github_base64 "$ref" "$dest"; then
        status="downloaded"
      else
        rc=$?
        if [ "$rc" -eq 2 ]; then
          status="skipped"
        else
          status="failed"
          fail=1
        fi
      fi
    fi
  fi

  # Post-download verification: delete a bad artifact and report loudly.
  if { [ "$status" = "downloaded" ] || [ "$status" = "cached" ]; } \
       && ! verify "$dest" "$bytes" "$sha"; then
    rm -f "$dest"
    log "ERROR: verification failed for $name"
    log "       expected size=$bytes sha256=$sha"
    log "       file deleted; aborting"
    fail=1
    status="mismatch"
  fi

  if [ -f "$dest" ]; then
    rows+=("$(printf '%-22s %10s  %s  %s  %s' "$name" "$(size_of "$dest")" "$(sha256_of "$dest")" "$src" "$status")")
  else
    rows+=("$(printf '%-22s %10s  %-64s  %s  %s' "$name" "$bytes" "$sha" "$src" "$status")")
  fi
done

printf '\n' >&2
printf '%-22s %10s  %-64s  %s\n' "NAME" "BYTES" "SHA256" "SOURCE / STATUS" >&2
printf '%s\n' "${rows[@]}" >&2

if [ "$fail" -ne 0 ]; then
  log ""
  log "One or more samples failed verification or download."
  exit 1
fi

log ""
log "All ${#SAMPLES[@]} curated samples are present and match their recorded hashes."
log "Reminder: these files are scratch test inputs, not repository fixtures and"
log "not authorization/compatibility evidence."
exit 0
