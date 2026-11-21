#!/usr/bin/env bash
# Compiles the widget's GLSL shaders into Qt shader packs (.qsb) for every
# graphics backend Qt Quick may use (OpenGL/ES, Vulkan, Direct3D, Metal).
# The .qsb files are committed; rerun this after editing a .frag.
set -euo pipefail

readonly PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
readonly SHADERS="$PROJECT_DIR/packaging/plasma/org.jambalinux.soniccore/contents/shaders"
QSB="${QSB:-$(command -v qsb6 || command -v qsb || true)}"
[[ -n "$QSB" ]] || QSB=/usr/lib/qt6/bin/qsb
[[ -x "$QSB" ]] || { printf 'erro: qsb (Qt Shader Tools) não encontrado\n' >&2; exit 1; }

for source in "$SHADERS"/*.frag; do
    "$QSB" --glsl "100es,120,150" --hlsl 50 --msl 12 -o "$source.qsb" "$source"
done
printf 'Shaders compilados em %s\n' "$SHADERS"
