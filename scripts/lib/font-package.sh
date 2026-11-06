#!/usr/bin/env bash
# font-package.sh — one staging implementation for every host's bundled fonts.
#
# `scripts/fetch-fonts.sh` assembles the *content* (mlightcad catalogue + the
# committed `fonts/` package); this library is the *policy* every build and
# packaging entry point shares, so no carrier can quietly ship fewer faces:
#
#   WITH_FONTS=1 (default)  scripts/fetch-fonts.sh downloads the full catalogue
#                           into <dest> (needs network)
#   WITH_FONTS=0            offline: copy the committed `fonts/` only
#   FONTS_DIR=<dir>         reuse a prepared directory (must contain fonts.json);
#                           overrides WITH_FONTS and performs no download
#
# `FONT_BASE_URL` / `FONT_FALLBACK_BASE_URL` / `FONT_RETRIES` are inherited by
# `fetch-fonts.sh` and are not reinterpreted here. `FONT_CACHE_DIR` is inherited
# too: downloads are cached per file (default `~/.cache/yacr/fonts`), so repeated
# packaging and the two Web bundles reuse one download.
#
# This file is **sourced, never executed**. The caller must set `YACR_REPO_ROOT`
# (absolute) and `YACR_SCRIPT_NAME` (diagnostic prefix) before sourcing.
#
# Contract exposed to callers:
#   yacr_stage_fonts <dest> [<label>]
#       Stages a complete package into <dest>: `fonts.json`, the face bytes and
#       the committed license texts. Returns non-zero (caller prints its own
#       diagnostic/`die`) when an input is missing, the selection is invalid or
#       no face landed. Never reports an empty/partial package as ready.
#   yacr_font_package_faces <dir>
#       Prints the number of face files at the top level of <dir>.
#
# Honesty rule: a missing toolchain/input is a loud non-zero exit. A staged
# package is build-input evidence only — not a font-rendering or host acceptance.
# shellcheck shell=bash

: "${YACR_REPO_ROOT:?font-package.sh: set YACR_REPO_ROOT before sourcing}"
: "${YACR_SCRIPT_NAME:=font-package}"

# Face extensions the engine can parse (SHX is a separate, non-sfnt path).
yacr_font_package_faces() {
    local dir="${1:?yacr_font_package_faces <dir>}"
    [ -d "$dir" ] || {
        printf '0\n'
        return 0
    }
    find "$dir" -maxdepth 1 -type f \
        \( -name '*.ttf' -o -name '*.otf' -o -name '*.ttc' \
        -o -name '*.woff' -o -name '*.woff2' -o -name '*.shx' \) \
        -printf '.' | wc -c
}

yacr_stage_fonts() {
    local dest="${1:?usage: yacr_stage_fonts <dest> [<label>]}"
    local label="${2:-fonts}"
    local with="${WITH_FONTS:-1}"
    case "$with" in
        0 | 1) ;;
        *)
            printf '%s: WITH_FONTS must be 0 or 1 (got %s)\n' "$YACR_SCRIPT_NAME" "'$with'" >&2
            return 1
            ;;
    esac

    mkdir -p -- "$dest" || return 1

    if [ -n "${FONTS_DIR:-}" ]; then
        [ -d "$FONTS_DIR" ] || {
            printf '%s: FONTS_DIR does not exist: %s\n' "$YACR_SCRIPT_NAME" "$FONTS_DIR" >&2
            return 1
        }
        [ -f "$FONTS_DIR/fonts.json" ] || {
            printf '%s: FONTS_DIR must contain fonts.json: %s\n' "$YACR_SCRIPT_NAME" "$FONTS_DIR" >&2
            return 1
        }
        printf '==> %s: using the prepared package %s (no download)\n' "$label" "$FONTS_DIR"
        cp -a -- "$FONTS_DIR/." "$dest/" || return 1
    elif [ "$with" = "1" ]; then
        [ -x "$YACR_REPO_ROOT/scripts/fetch-fonts.sh" ] || {
            printf '%s: missing executable scripts/fetch-fonts.sh\n' "$YACR_SCRIPT_NAME" >&2
            return 1
        }
        printf '==> %s: assembling the full CAD catalogue (scripts/fetch-fonts.sh)\n' "$label"
        "$YACR_REPO_ROOT/scripts/fetch-fonts.sh" "$dest" || return 1
    else
        [ -f "$YACR_REPO_ROOT/fonts/fonts.json" ] || {
            printf '%s: committed fonts/fonts.json is missing\n' "$YACR_SCRIPT_NAME" >&2
            return 1
        }
        printf '==> %s: WITH_FONTS=0, staging the committed fonts/ only (offline)\n' "$label"
        cp -a -- "$YACR_REPO_ROOT/fonts/." "$dest/" || return 1
    fi

    [ -s "$dest/fonts.json" ] || {
        printf '%s: %s/fonts.json is missing or empty\n' "$YACR_SCRIPT_NAME" "$dest" >&2
        return 1
    }
    local faces
    faces="$(yacr_font_package_faces "$dest")"
    [ "${faces:-0}" -gt 0 ] || {
        printf '%s: no font face files landed in %s\n' "$YACR_SCRIPT_NAME" "$dest" >&2
        return 1
    }
    printf '  %-28s %s face(s)\n' "$label" "$faces"
}
