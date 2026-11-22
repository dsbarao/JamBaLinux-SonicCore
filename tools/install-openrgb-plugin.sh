#!/usr/bin/env bash
# Builds and installs the JamBaLinux Sonic Core plugin for OpenRGB, which makes
# the JBL Quantum 810 Wireless appear as an OpenRGB device. The plugin drives
# the headset only through the soniccore CLI, so install SonicCore first
# (tools/install-user.sh).
#
# A plugin must be compiled against the headers of the installed OpenRGB
# version. Pass an OpenRGB source tree with --openrgb-source, or let this
# script clone the matching release tag from the official repository into a
# temporary directory (removed afterwards).
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
readonly PLUGIN_SOURCE="$PROJECT_DIR/openrgb-plugin"
readonly PLUGIN_FILE="libJamBaLinuxSonicCorePlugin.so"
readonly PLUGIN_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/OpenRGB/plugins"
readonly OPENRGB_REPOSITORY="https://gitlab.com/CalcProgrammer1/OpenRGB.git"
# OpenRGB versions whose plugin API (5) this plugin implements.
readonly SUPPORTED_OPENRGB_VERSIONS="1.1"

fail() {
    printf 'erro: %s\n' "$*" >&2
    exit 1
}

usage() {
    printf '%s\n' 'uso: tools/install-openrgb-plugin.sh [--check] [--openrgb-source DIR]' \
        '       tools/install-openrgb-plugin.sh --uninstall' >&2
    exit 2
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "comando obrigatório não encontrado: $1"
}

openrgb_version() {
    openrgb --version 2>/dev/null | sed -n 's/^[[:space:]]*Version:[[:space:]]*//p' | head -n 1
}

check_prerequisites() {
    require_command openrgb
    require_command qmake6
    require_command make
    require_command g++
    local version
    version="$(openrgb_version)"
    [[ -n "$version" ]] || fail "não foi possível ler a versão do OpenRGB (openrgb --version)"
    [[ " $SUPPORTED_OPENRGB_VERSIONS " == *" $version "* ]] \
        || fail "OpenRGB $version não é suportado; versões suportadas: $SUPPORTED_OPENRGB_VERSIONS"
    printf '%s\n' "$version"
}

check=0
uninstall=0
openrgb_source=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --check) check=1 ;;
        --uninstall) uninstall=1 ;;
        --openrgb-source)
            [[ $# -ge 2 ]] || usage
            openrgb_source="$2"
            shift
            ;;
        *) usage ;;
    esac
    shift
done

if [[ "$uninstall" -eq 1 ]]; then
    [[ "$check" -eq 0 && -z "$openrgb_source" ]] || usage
    rm -f -- "$PLUGIN_DIR/$PLUGIN_FILE"
    printf 'Plugin removido de %s. Reinicie o OpenRGB.\n' "$PLUGIN_DIR"
    exit 0
fi

version="$(check_prerequisites)"
if [[ -n "$openrgb_source" ]]; then
    [[ -f "$openrgb_source/OpenRGBPluginInterface.h" ]] \
        || fail "$openrgb_source não é uma árvore de código do OpenRGB"
else
    require_command git
fi

if [[ "$check" -eq 1 ]]; then
    printf 'OpenRGB %s encontrado; o plugin seria instalado em %s.\n' "$version" "$PLUGIN_DIR/$PLUGIN_FILE"
    [[ -n "$openrgb_source" ]] \
        || printf 'O código do OpenRGB (tag release_%s) seria baixado de %s.\n' "$version" "$OPENRGB_REPOSITORY"
    printf '%s\n' 'Nenhuma alteração foi feita.'
    exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

if [[ -z "$openrgb_source" ]]; then
    printf 'Baixando o código do OpenRGB %s (só os cabeçalhos são usados)...\n' "$version"
    git clone --quiet --depth 1 --branch "release_$version" "$OPENRGB_REPOSITORY" "$work/OpenRGB"
    openrgb_source="$work/OpenRGB"
fi

mkdir -p -- "$work/build"
(
    cd -- "$work/build"
    qmake6 "OPENRGB_DIR=$openrgb_source" "$PLUGIN_SOURCE/JamBaLinuxOpenRGBPlugin.pro" >/dev/null
    make -j"$(nproc)" >/dev/null
)
[[ -f "$work/build/$PLUGIN_FILE" ]] || fail "a compilação não gerou $PLUGIN_FILE"

install -Dm755 -- "$work/build/$PLUGIN_FILE" "$PLUGIN_DIR/$PLUGIN_FILE"
printf 'Plugin instalado em %s.\n' "$PLUGIN_DIR/$PLUGIN_FILE"
printf '%s\n' 'Reinicie o OpenRGB para carregar o plugin.'
