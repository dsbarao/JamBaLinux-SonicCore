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
readonly AUTOSTART_SOURCE="$PROJECT_DIR/packaging/kde/jambalinux-soniccore-autostart.desktop"
readonly AUTOSTART_TARGET="$HOME/.config/autostart/jambalinux-soniccore.desktop"
readonly APP_SOURCE="$PROJECT_DIR/packaging/app"
readonly APP_TARGET="$HOME/.local/share/jambalinux-soniccore/app"
readonly ICON_SOURCE="$PROJECT_DIR/packaging/brand/jambalinux-soniccore.svg"
readonly ICON_TARGET="$HOME/.local/share/icons/hicolor/scalable/apps/jambalinux-soniccore.svg"
readonly METER_SERVICE_SOURCE="$PROJECT_DIR/packaging/dbus/org.jambalinux.soniccore.Meter.service"
readonly METER_SERVICE_TARGET="${XDG_DATA_HOME:-$HOME/.local/share}/dbus-1/services/org.jambalinux.soniccore.Meter.service"
readonly CARGO_BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
readonly CARGO_BINARY="$CARGO_BIN_DIR/soniccore"
readonly DEFAULT_CARGO_BIN_DIR="$HOME/.cargo/bin"
readonly ACP_OVERLAY="$SCRIPT_DIR/acp-overlay.sh"
STAGE_DIR=""

fail() {
    printf 'erro: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "comando obrigatório não encontrado: $1"
}

check_prerequisites() {
    require_command cargo
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
    [[ -f "$AUTOSTART_SOURCE" ]] || fail "início automático do app não encontrado"
    [[ -f "$APP_SOURCE/main.qml" && -x "$APP_SOURCE/soniccore-app" ]] || fail "app SonicCore não encontrado"
    [[ -f "$METER_SERVICE_SOURCE" ]] || fail "ativação D-Bus do medidor não encontrada"
    [[ -x "$ACP_OVERLAY" ]] || fail "script do overlay ACP não encontrado"
    # The binary path is written into systemd units, a shell command and a
    # sed replacement: refuse characters that would need escaping there.
    [[ ! "$CARGO_BIN_DIR" =~ [[:space:]\'\"\\\$\`|%] ]] \
        || fail "CARGO_HOME com espaço, aspas, \\, \$, \`, | ou % não é suportado: $CARGO_BIN_DIR"
    # Refuse before changing anything: an overlay file edited outside
    # JamBaLinux would otherwise stop the install half-way.
    "$ACP_OVERLAY" --verify
}

cleanup_stage() {
    [[ -z "$STAGE_DIR" ]] || rm -rf -- "$STAGE_DIR"
}

# The packaged units, launcher and widget call ~/.cargo/bin. With a custom
# CARGO_HOME, install copies that point at the real binary instead.
stage_packaging() {
    PLASMOID_INSTALL="$PLASMOID_SOURCE"
    SERVICE_INSTALL="$SERVICE_SOURCE"
    EQUALIZER_SERVICE_INSTALL="$EQUALIZER_SERVICE_SOURCE"
    SPATIAL_SERVICE_INSTALL="$SPATIAL_SERVICE_SOURCE"
    LAUNCHER_INSTALL="$LAUNCHER_SOURCE"
    [[ "$CARGO_BIN_DIR" != "$DEFAULT_CARGO_BIN_DIR" ]] || return 0

    STAGE_DIR="$(mktemp -d)"
    trap cleanup_stage EXIT
    cp -a -- "$PLASMOID_SOURCE" "$STAGE_DIR/$PLASMOID_ID"
    PLASMOID_INSTALL="$STAGE_DIR/$PLASMOID_ID"
    sed -i "s|\$HOME/.cargo/bin/soniccore|$CARGO_BIN_DIR/soniccore|" \
        "$PLASMOID_INSTALL/contents/ui/SonicCore.qml" \
        "$PLASMOID_INSTALL/contents/ui/configBattery.qml" \
        "$PLASMOID_INSTALL/contents/ui/configOutput.qml"
    SERVICE_INSTALL="$STAGE_DIR/$(basename -- "$SERVICE_SOURCE")"
    EQUALIZER_SERVICE_INSTALL="$STAGE_DIR/$(basename -- "$EQUALIZER_SERVICE_SOURCE")"
    SPATIAL_SERVICE_INSTALL="$STAGE_DIR/$(basename -- "$SPATIAL_SERVICE_SOURCE")"
    sed "s|%h/.cargo/bin/|$CARGO_BIN_DIR/|g" "$SERVICE_SOURCE" > "$SERVICE_INSTALL"
    sed "s|%h/.cargo/bin/|$CARGO_BIN_DIR/|g" "$EQUALIZER_SERVICE_SOURCE" > "$EQUALIZER_SERVICE_INSTALL"
    sed "s|%h/.cargo/bin/|$CARGO_BIN_DIR/|g" "$SPATIAL_SERVICE_SOURCE" > "$SPATIAL_SERVICE_INSTALL"
    local staged
    for staged in "$PLASMOID_INSTALL/contents/ui/SonicCore.qml" \
        "$PLASMOID_INSTALL/contents/ui/configBattery.qml" \
        "$PLASMOID_INSTALL/contents/ui/configOutput.qml" "$SERVICE_INSTALL" \
        "$EQUALIZER_SERVICE_INSTALL" "$SPATIAL_SERVICE_INSTALL"; do
        grep -Fq "$CARGO_BIN_DIR/soniccore" "$staged" \
            || fail "não foi possível apontar $staged para $CARGO_BIN_DIR"
        ! grep -Fq '.cargo/bin' "$staged" || [[ "$CARGO_BIN_DIR" == *.cargo/bin* ]] \
            || fail "$staged ainda aponta para ~/.cargo/bin"
    done
}

if [[ "${1:-}" == "--check" ]]; then
    check_prerequisites
    stage_packaging
    "$ACP_OVERLAY" --check
    printf 'Pré-requisitos do JamBaLinux Sonic Core verificados. Nenhuma alteração foi feita.\n'
    exit 0
fi

[[ $# -eq 0 ]] || fail "uso: tools/install-user.sh [--check]"
check_prerequisites
stage_packaging

cd -- "$PROJECT_DIR"
cargo install --path . --bins --force
[[ -x "$CARGO_BINARY" ]] || fail "soniccore não foi encontrado em $CARGO_BINARY após a instalação"

# SonicCore is an app now; the Plasma widget it replaced is taken off the
# panels and uninstalled. Tray icons are capped at 22 px, the widget is gone.
remove_old_widget() {
    [[ -d "$PLASMOID_TARGET" ]] || return 0
    local qdbus
    qdbus="$(command -v qdbus6 || command -v qdbus || true)"
    if [[ -n "$qdbus" ]]; then
        "$qdbus" org.kde.plasmashell /PlasmaShell org.kde.PlasmaShell.evaluateScript \
            "panels().concat(desktops()).forEach(c => c.widgets('$PLASMOID_ID').forEach(w => w.remove()))" \
            >/dev/null 2>&1 || printf 'aviso: remova o widget antigo do painel manualmente.\n' >&2
    fi
    if command -v kpackagetool6 >/dev/null 2>&1; then
        kpackagetool6 --type Plasma/Applet --remove "$PLASMOID_ID" >/dev/null 2>&1 || true
    fi
    rm -rf -- "$PLASMOID_TARGET"
}
remove_old_widget

install -Dm644 "$SERVICE_INSTALL" "$SERVICE_TARGET"
install -Dm644 "$EQUALIZER_SERVICE_INSTALL" "$EQUALIZER_SERVICE_TARGET"
install -Dm644 "$SPATIAL_SERVICE_INSTALL" "$SPATIAL_SERVICE_TARGET"
install -Dm644 "$LAUNCHER_INSTALL" "$LAUNCHER_TARGET"
install -Dm644 "$AUTOSTART_SOURCE" "$AUTOSTART_TARGET"
install -Dm644 "$ICON_SOURCE" "$ICON_TARGET"
# The app carries its own copy of the interface (QML, shaders, images).
rm -rf -- "$APP_TARGET"
install -Dm755 "$APP_SOURCE/soniccore-app" "$APP_TARGET/soniccore-app"
install -Dm644 "$ICON_SOURCE" "$APP_TARGET/jambalinux-soniccore.svg"
sed "s|^import \"../plasma/$PLASMOID_ID/contents/ui\"\$|import \"contents/ui\"|" \
    "$APP_SOURCE/main.qml" > "$APP_TARGET/main.qml"
grep -Fqx 'import "contents/ui"' "$APP_TARGET/main.qml" \
    || fail "não foi possível apontar o app para a interface instalada"
cp -r -- "$APP_SOURCE/icons" "$APP_TARGET/icons"
cp -r -- "$PLASMOID_INSTALL/contents" "$APP_TARGET/contents"
# KWin remembers the window's place, size and monitor (Wayland apps cannot).
"$SCRIPT_DIR/kwin-window-rule.sh" --install || printf 'aviso: não foi possível criar a regra de janela do KWin.\n' >&2
# D-Bus activation needs an absolute Exec path (the path was validated above).
mkdir -p -- "$(dirname -- "$METER_SERVICE_TARGET")"
sed "s|@SONICCORE@|$CARGO_BINARY|" "$METER_SERVICE_SOURCE" > "$METER_SERVICE_TARGET"
chmod 644 -- "$METER_SERVICE_TARGET"
# The session bus reads activation files at start-up; ask it to look again.
if command -v busctl >/dev/null 2>&1; then
    busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true
fi
systemctl --user daemon-reload
systemctl --user enable jambalinux-soniccore.service
systemctl --user enable jambalinux-soniccore-equalizer.service
systemctl --user enable jambalinux-soniccore-spatial.service
# Installation is the controlled deployment boundary: restart once here so an
# already-running service cannot keep an older binary. Band changes never use
# systemctl and update the persistent equalizer node in place.
systemctl --user restart jambalinux-soniccore.service
# Stop spatial while the equalizer still exists: on SIGTERM it moves streams to
# the equalizer sink and fails if that sink is already gone.
systemctl --user stop jambalinux-soniccore-spatial.service
systemctl --user restart jambalinux-soniccore-equalizer.service
systemctl --user start jambalinux-soniccore-spatial.service
# The ACP overlay only takes effect when WirePlumber recreates the card; the
# installer does not restart it, so audio is never cut without being asked.
"$ACP_OVERLAY" --install

printf '\nJamBaLinux Sonic Core instalado para o usuário atual.\n'
printf 'O app abre pelo menu (JamBaLinux Sonic Core) e inicia na bandeja ao entrar na sessão.\n'
command -v qml6 >/dev/null 2>&1 || command -v qml >/dev/null 2>&1 \
    || printf 'aviso: qml6 não encontrado; instale o Qt Declarative para abrir o app.\n' >&2
printf 'O Plasma não foi reiniciado automaticamente.\n'
printf 'A regra udev privilegiada também não foi alterada. Consulte o README se houver erro de permissão.\n'
