#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
readonly PLASMOID_ID="org.jambalinux.soniccore"
readonly PLASMOID_SOURCE="$PROJECT_DIR/packaging/plasma/$PLASMOID_ID"
readonly PLASMOID_TARGET="$HOME/.local/share/plasma/plasmoids/$PLASMOID_ID"
readonly SERVICE_SOURCE="$PROJECT_DIR/packaging/systemd/jambalinux-soniccore.service"
readonly SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore.service"
readonly EQUALIZER_SERVICE_SOURCE="$PROJECT_DIR/packaging/systemd/jambalinux-soniccore-equalizer.service"
readonly EQUALIZER_SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore-equalizer.service"
readonly SPATIAL_SERVICE_SOURCE="$PROJECT_DIR/packaging/systemd/jambalinux-soniccore-spatial.service"
readonly SPATIAL_SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore-spatial.service"
readonly LAUNCHER_SOURCE="$PROJECT_DIR/packaging/kde/jambalinux-soniccore.desktop"
readonly LAUNCHER_TARGET="$HOME/.local/share/applications/jambalinux-soniccore.desktop"
readonly CARGO_BINARY="${CARGO_HOME:-$HOME/.cargo}/bin/soniccore"

fail() {
    printf 'erro: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "comando obrigatório não encontrado: $1"
}

check_prerequisites() {
    require_command cargo
    require_command kpackagetool6
    require_command pactl
    require_command pipewire
    require_command pw-link
    require_command pw-cli
    require_command pw-dump
    require_command systemctl
    [[ -f "$PROJECT_DIR/Cargo.toml" ]] || fail "Cargo.toml não encontrado"
    [[ -f "$PLASMOID_SOURCE/metadata.json" ]] || fail "pacote Plasma não encontrado"
    [[ -f "$SERVICE_SOURCE" ]] || fail "serviço systemd não encontrado"
    [[ -f "$EQUALIZER_SERVICE_SOURCE" ]] || fail "serviço systemd do equalizador não encontrado"
    [[ -f "$SPATIAL_SERVICE_SOURCE" ]] || fail "serviço systemd espacial não encontrado"
    [[ -f "$LAUNCHER_SOURCE" ]] || fail "lançador KDE não encontrado"
}

if [[ "${1:-}" == "--check" ]]; then
    check_prerequisites
    printf 'Pré-requisitos do JamBaLinux SonicCore verificados. Nenhuma alteração foi feita.\n'
    exit 0
fi

[[ $# -eq 0 ]] || fail "uso: tools/install-user.sh [--check]"
check_prerequisites

cd -- "$PROJECT_DIR"
cargo install --path . --bins --force
[[ -x "$CARGO_BINARY" ]] || fail "soniccore não foi encontrado em $CARGO_BINARY após a instalação"

if [[ -d "$PLASMOID_TARGET" ]]; then
    kpackagetool6 --type Plasma/Applet --upgrade "$PLASMOID_SOURCE"
else
    kpackagetool6 --type Plasma/Applet --install "$PLASMOID_SOURCE"
fi

install -Dm644 "$SERVICE_SOURCE" "$SERVICE_TARGET"
install -Dm644 "$EQUALIZER_SERVICE_SOURCE" "$EQUALIZER_SERVICE_TARGET"
install -Dm644 "$SPATIAL_SERVICE_SOURCE" "$SPATIAL_SERVICE_TARGET"
install -Dm644 "$LAUNCHER_SOURCE" "$LAUNCHER_TARGET"
systemctl --user daemon-reload
systemctl --user enable jambalinux-soniccore.service
systemctl --user enable jambalinux-soniccore-equalizer.service
systemctl --user enable jambalinux-soniccore-spatial.service
# Installation is the controlled deployment boundary: restart once here so an
# already-running service cannot keep an older binary. Band changes never use
# systemctl and update the persistent equalizer node in place.
systemctl --user restart jambalinux-soniccore.service
systemctl --user restart jambalinux-soniccore-equalizer.service
systemctl --user restart jambalinux-soniccore-spatial.service

printf '\nJamBaLinux SonicCore instalado para o usuário atual.\n'
printf 'O Plasma não foi reiniciado automaticamente.\n'
printf 'A regra udev privilegiada também não foi alterada. Consulte o README se houver erro de permissão.\n'
