#!/usr/bin/env bash
# Builds the JamBaLinux Sonic Core app as a Flatpak and installs it for the
# current user. For distributions whose Qt is older than 6.5: Ubuntu 24.04,
# Pop!_OS 24.04 (COSMIC), Linux Mint 22 (Cinnamon). The KDE runtime (Qt 6 and
# Kirigami, about 1 GB the first time) comes from Flathub.
#
#   tools/build-flatpak.sh            build and install
#   tools/build-flatpak.sh --bundle F also write a single-file bundle F
#
# tools/install-user.sh --flatpak runs this after installing the core.
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
readonly APP_ID="io.github.dsbarao.SonicCore"
readonly MANIFEST="$PROJECT_DIR/packaging/flatpak/$APP_ID.yml"
readonly WORK_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/jambalinux-soniccore/flatpak"

fail() {
    printf 'erro: %s\n' "$*" >&2
    exit 1
}

bundle=""
case "${1:-}" in
    "") ;;
    --bundle)
        [[ $# -eq 2 ]] || fail "uso: tools/build-flatpak.sh [--bundle ARQUIVO]"
        bundle="$2"
        ;;
    *) fail "uso: tools/build-flatpak.sh [--bundle ARQUIVO]" ;;
esac

command -v flatpak >/dev/null 2>&1 \
    || fail "flatpak não encontrado; instale-o (sudo apt install flatpak) e entre na sessão de novo"
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo

# flatpak-builder from the distribution, or the one from Flathub.
if command -v flatpak-builder >/dev/null 2>&1; then
    builder=(flatpak-builder)
else
    flatpak info --user org.flatpak.Builder >/dev/null 2>&1 \
        || flatpak install --user -y --noninteractive flathub org.flatpak.Builder
    builder=(flatpak run --filesystem="$PROJECT_DIR" --filesystem="$WORK_DIR" org.flatpak.Builder)
fi

mkdir -p -- "$WORK_DIR"
"${builder[@]}" --user --install-deps-from=flathub --force-clean --install \
    --state-dir="$WORK_DIR/state" --repo="$WORK_DIR/repo" \
    "$WORK_DIR/build" "$MANIFEST"

if [[ -n "$bundle" ]]; then
    flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
        "$WORK_DIR/repo" "$bundle" "$APP_ID"
    printf 'Pacote único gravado em %s.\n' "$bundle"
fi
printf 'App %s instalado (Flatpak). Abra pelo menu: JamBaLinux Sonic Core.\n' "$APP_ID"
