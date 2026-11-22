#!/usr/bin/env bash
# Renders the JamBaLinux Sonic Core brand from packaging/brand: the wordmark
# PNG the app shows at the top (its text uses Fira Sans, so the PNG, not the
# SVG, is what ships). The icon SVG is installed as it is. Rerun after
# editing packaging/brand/*.svg; the PNGs are committed.
set -euo pipefail

readonly PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
readonly BRAND="$PROJECT_DIR/packaging/brand"
readonly IMAGES="$PROJECT_DIR/packaging/app/contents/images"

command -v rsvg-convert >/dev/null 2>&1 || { printf 'erro: rsvg-convert (librsvg) não encontrado\n' >&2; exit 1; }
fc-list : family | grep "Fira Sans Compressed" >/dev/null \
    || { printf 'erro: a fonte Fira Sans (Compressed/Condensed) é necessária para o letreiro\n' >&2; exit 1; }

rsvg-convert -w 760 "$BRAND/wordmark.svg" -o "$BRAND/wordmark.png"
rsvg-convert -w 1520 "$BRAND/wordmark.svg" -o "$BRAND/wordmark@2x.png"
cp -- "$BRAND/wordmark@2x.png" "$IMAGES/wordmark.png"
rsvg-convert -w 256 "$BRAND/jambalinux-soniccore.svg" -o "$BRAND/jambalinux-soniccore-256.png"
printf 'Marca gerada em %s\n' "$BRAND"
