#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly ACP_OVERLAY="$SCRIPT_DIR/acp-overlay.sh"
readonly PLASMOID_ID="org.jambalinux.soniccore"
readonly PLASMOID_TARGET="$HOME/.local/share/plasma/plasmoids/$PLASMOID_ID"
readonly SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore.service"
readonly EQUALIZER_SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore-equalizer.service"
readonly SPATIAL_SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore-spatial.service"
readonly LIGHTS_SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore-lights.service"
readonly LAUNCHER_TARGET="$HOME/.local/share/applications/jambalinux-soniccore.desktop"
readonly AUTOSTART_TARGET="$HOME/.config/autostart/jambalinux-soniccore.desktop"
readonly APP_TARGET="$HOME/.local/share/jambalinux-soniccore/app"
readonly ICON_TARGET="$HOME/.local/share/icons/hicolor/scalable/apps/jambalinux-soniccore.svg"
readonly CARGO_BINARY="${CARGO_HOME:-$HOME/.cargo}/bin/soniccore"
readonly METER_SERVICE_TARGET="${XDG_DATA_HOME:-$HOME/.local/share}/dbus-1/services/org.jambalinux.soniccore.Meter.service"
readonly OPENRGB_PLUGIN="${XDG_CONFIG_HOME:-$HOME/.config}/OpenRGB/plugins/libJamBaLinuxSonicCorePlugin.so"

show_plan() {
    printf '%s\n' 'Componentes do usuário que serão removidos:'
    printf '  %s\n' "$PLASMOID_TARGET"
    printf '  %s\n' "$SERVICE_TARGET"
    printf '  %s\n' "$EQUALIZER_SERVICE_TARGET"
    printf '  %s\n' "$SPATIAL_SERVICE_TARGET"
    printf '  %s\n' "$LIGHTS_SERVICE_TARGET"
    printf '  %s\n' "$LAUNCHER_TARGET"
    printf '  %s\n' "$AUTOSTART_TARGET"
    printf '  %s\n' "$APP_TARGET"
    printf '  %s\n' "$ICON_TARGET"
    printf '  %s\n' "$CARGO_BINARY"
    printf '  %s\n' "$METER_SERVICE_TARGET"
    printf '  %s (plugin do OpenRGB, se instalado)\n' "$OPENRGB_PLUGIN"
    "$ACP_OVERLAY" --check
    printf '%s\n' 'Não serão removidos: repositório, capturas, documentação ou regra udev.'
}

if [[ "${1:-}" == "--check" ]]; then
    show_plan
    printf '%s\n' 'Prévia concluída. Nenhuma alteração foi feita.'
    exit 0
fi

if [[ "${1:-}" != "--confirm" || $# -ne 1 ]]; then
    printf '%s\n' 'uso: tools/uninstall-user.sh --check | --confirm' >&2
    exit 2
fi

show_plan

# The PC's lights go back to OpenRGB (and its profile) before the engine
# that drew them stops.
if [[ -x "$CARGO_BINARY" ]]; then
    "$CARGO_BINARY" lights off >/dev/null 2>&1 && sleep 1 || true
fi
systemctl --user disable --now jambalinux-soniccore-lights.service 2>/dev/null || true
# Stop spatial first: its shutdown restores streams to the equalizer sink.
systemctl --user disable --now jambalinux-soniccore-spatial.service 2>/dev/null || true
systemctl --user disable --now jambalinux-soniccore-equalizer.service 2>/dev/null || true
systemctl --user disable --now jambalinux-soniccore.service 2>/dev/null || true

if command -v kpackagetool6 >/dev/null 2>&1 && [[ -d "$PLASMOID_TARGET" ]]; then
    kpackagetool6 --type Plasma/Applet --remove "$PLASMOID_ID"
fi

rm -rf -- "$APP_TARGET"
# The app as a Flatpak (tools/install-user.sh --flatpak).
if command -v flatpak >/dev/null 2>&1 && flatpak info --user io.github.dsbarao.SonicCore >/dev/null 2>&1; then
    flatpak uninstall --user -y --noninteractive io.github.dsbarao.SonicCore >/dev/null || true
fi
rm -f -- "$ICON_TARGET"
if command -v kwriteconfig6 >/dev/null 2>&1; then
    "$SCRIPT_DIR/kwin-window-rule.sh" --uninstall || true
fi
rm -f -- "$SERVICE_TARGET" "$EQUALIZER_SERVICE_TARGET" "$SPATIAL_SERVICE_TARGET" "$LIGHTS_SERVICE_TARGET" "$LAUNCHER_TARGET" "$AUTOSTART_TARGET" "$METER_SERVICE_TARGET" "$OPENRGB_PLUGIN"
# Files edited outside JamBaLinux are kept and reported; the rest still goes.
overlay_kept=0
"$ACP_OVERLAY" --uninstall || overlay_kept=1
systemctl --user daemon-reload

if command -v cargo >/dev/null 2>&1 && [[ -x "$CARGO_BINARY" ]]; then
    cargo uninstall jambalinux-soniccore
fi

# The app (and a copy from before it had its own launcher).
pkill -u "$(id -u)" -f -- "$APP_TARGET/soniccore-app" 2>/dev/null || true
pkill -u "$(id -u)" -f -- "$APP_TARGET/main.qml" 2>/dev/null || true
printf '%s\n' 'Reinicie o Plasma manualmente se o widget antigo ainda aparecer no painel.'
if [[ "$overlay_kept" -ne 0 ]]; then
    printf '%s\n' 'Remoção parcial: arquivos do overlay ACP modificados fora do JamBaLinux foram mantidos (veja acima).' >&2
    exit 1
fi
printf '%s\n' 'Componentes do JamBaLinux Sonic Core instalados para o usuário foram removidos.'
