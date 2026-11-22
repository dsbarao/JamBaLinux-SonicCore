#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
# The Plasma widget this app replaced; only removed if an old install left it.
readonly PLASMOID_ID="org.jambalinux.soniccore"
readonly PLASMOID_TARGET="$HOME/.local/share/plasma/plasmoids/$PLASMOID_ID"
readonly SERVICE_SOURCE="$PROJECT_DIR/packaging/systemd/jambalinux-soniccore.service"
readonly SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore.service"
readonly EQUALIZER_SERVICE_SOURCE="$PROJECT_DIR/packaging/systemd/jambalinux-soniccore-equalizer.service"
readonly EQUALIZER_SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore-equalizer.service"
readonly SPATIAL_SERVICE_SOURCE="$PROJECT_DIR/packaging/systemd/jambalinux-soniccore-spatial.service"
readonly SPATIAL_SERVICE_TARGET="$HOME/.config/systemd/user/jambalinux-soniccore-spatial.service"
readonly LAUNCHER_SOURCE="$PROJECT_DIR/packaging/desktop/jambalinux-soniccore.desktop"
readonly LAUNCHER_TARGET="$HOME/.local/share/applications/jambalinux-soniccore.desktop"
readonly AUTOSTART_SOURCE="$PROJECT_DIR/packaging/desktop/jambalinux-soniccore-autostart.desktop"
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
# The interface needs Qt 6.5 or newer (MultiEffect, shader packs) and KDE's
# Kirigami 6 at run time. Ubuntu 24.04 and what is built on it (Pop!_OS
# 24.04, Mint 22) ship Qt 6.4; see the README for those.
readonly MINIMUM_QT="6.5"
# let-chains and slice::as_chunks (Rust 1.88); Ubuntu 24.04's cargo is 1.75,
# install Rust with rustup there.
readonly MINIMUM_RUST="1.88"
readonly FLATPAK_APP_ID="io.github.dsbarao.SonicCore"
FLATPAK_MODE=0
STAGE_DIR=""
BUILD_DIR=""
QMAKE=""

fail() {
    printf 'erro: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "comando obrigatório não encontrado: $1"
}

# qmake from Qt 6, by any of the names distributions give it.
find_qmake() {
    local candidate
    for candidate in qmake6 qmake-qt6 /usr/lib64/qt6/bin/qmake /usr/lib/qt6/bin/qmake \
        /usr/lib/x86_64-linux-gnu/qt6/bin/qmake; do
        if command -v "$candidate" >/dev/null 2>&1 \
            && [[ "$("$candidate" -query QT_VERSION 2>/dev/null)" == 6.* ]]; then
            command -v "$candidate"
            return 0
        fi
    done
    return 1
}

# version_at_least HAVE NEED: dotted numbers, compared part by part (plain
# bash, so it works with a minimal PATH).
version_at_least() {
    local -a have need
    IFS=. read -r -a have <<< "$1"
    IFS=. read -r -a need <<< "$2"
    local index
    for index in 0 1 2; do
        local a="${have[index]:-0}" b="${need[index]:-0}"
        a="${a%%[!0-9]*}" b="${b%%[!0-9]*}"
        (( ${a:-0} > ${b:-0} )) && return 0
        (( ${a:-0} < ${b:-0} )) && return 1
    done
    return 0
}

check_qt() {
    QMAKE="$(find_qmake)" || fail "qmake do Qt 6 não encontrado; instale as ferramentas de desenvolvimento do Qt 6 (veja o README) ou use tools/install-user.sh --flatpak"
    local version
    version="$("$QMAKE" -query QT_VERSION)"
    version_at_least "$version" "$MINIMUM_QT" \
        || fail "Qt $version é antigo demais para a interface (precisa do $MINIMUM_QT ou mais novo). No Ubuntu 24.04, Pop!_OS 24.04 e Mint 22 use: tools/install-user.sh --flatpak"
    require_command make
    command -v c++ >/dev/null 2>&1 || command -v g++ >/dev/null 2>&1 \
        || fail "compilador C++ não encontrado (g++ ou clang++)"
}

# Run-time QML modules the interface imports; missing ones only warn, since
# they can be installed afterwards without reinstalling.
check_qml_modules() {
    local qml_dir module missing=()
    qml_dir="$("$QMAKE" -query QT_INSTALL_QML 2>/dev/null || true)"
    for module in org/kde/kirigami QtQuick/Controls QtQuick/Effects QtQuick/Shapes QtCore Qt/labs/platform QtMultimedia; do
        [[ -f "$qml_dir/$module/qmldir" ]] || missing+=("${module//\//.}")
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        printf 'aviso: módulos QML não encontrados em %s: %s\n' "$qml_dir" "${missing[*]}" >&2
        printf 'aviso: o app só abre depois de instalá-los (veja o README).\n' >&2
    fi
}

check_rust() {
    local version
    version="$(cargo --version 2>/dev/null || true)"
    version="${version#cargo }"
    version="${version%% *}"
    version_at_least "${version:-0}" "$MINIMUM_RUST" \
        || fail "Rust ${version:-?} é antigo demais (precisa do $MINIMUM_RUST ou mais novo); instale pelo rustup: https://rustup.rs"
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
    [[ -f "$SERVICE_SOURCE" ]] || fail "serviço systemd não encontrado"
    [[ -f "$EQUALIZER_SERVICE_SOURCE" ]] || fail "serviço systemd do equalizador não encontrado"
    [[ -f "$SPATIAL_SERVICE_SOURCE" ]] || fail "serviço systemd espacial não encontrado"
    [[ -f "$LAUNCHER_SOURCE" ]] || fail "atalho do menu não encontrado"
    [[ -f "$AUTOSTART_SOURCE" ]] || fail "início automático do app não encontrado"
    [[ -f "$APP_SOURCE/main.qml" && -f "$APP_SOURCE/launcher/soniccore-app.pro" ]] || fail "app SonicCore não encontrado"
    if [[ "$FLATPAK_MODE" -eq 1 ]]; then
        require_command flatpak
        # The Flatpak calls the CLI at ~/.cargo/bin, where cargo puts it.
        [[ "$CARGO_BIN_DIR" == "$DEFAULT_CARGO_BIN_DIR" ]] \
            || fail "--flatpak usa o soniccore em ~/.cargo/bin; instale sem CARGO_HOME personalizado"
    fi
    [[ -f "$METER_SERVICE_SOURCE" ]] || fail "ativação D-Bus do medidor não encontrada"
    [[ -x "$ACP_OVERLAY" ]] || fail "script do overlay ACP não encontrado"
    # The binary path is written into systemd units, a shell command and a
    # sed replacement: refuse characters that would need escaping there.
    [[ ! "$CARGO_BIN_DIR" =~ [[:space:]\'\"\\\$\`|%] ]] \
        || fail "CARGO_HOME com espaço, aspas, \\, \$, \`, | ou % não é suportado: $CARGO_BIN_DIR"
    check_rust
    [[ "$FLATPAK_MODE" -eq 1 ]] || check_qt
    # Refuse before changing anything: an overlay file edited outside
    # JamBaLinux would otherwise stop the install half-way.
    "$ACP_OVERLAY" --verify
}

cleanup_stage() {
    [[ -z "$STAGE_DIR" ]] || rm -rf -- "$STAGE_DIR"
    [[ -z "$BUILD_DIR" ]] || rm -rf -- "$BUILD_DIR"
}
trap cleanup_stage EXIT

# The packaged units and the app's interface call ~/.cargo/bin. With a custom
# CARGO_HOME, install copies that point at the real binary instead.
stage_packaging() {
    CONTENTS_INSTALL="$APP_SOURCE/contents"
    SERVICE_INSTALL="$SERVICE_SOURCE"
    EQUALIZER_SERVICE_INSTALL="$EQUALIZER_SERVICE_SOURCE"
    SPATIAL_SERVICE_INSTALL="$SPATIAL_SERVICE_SOURCE"
    LAUNCHER_INSTALL="$LAUNCHER_SOURCE"
    [[ "$CARGO_BIN_DIR" != "$DEFAULT_CARGO_BIN_DIR" ]] || return 0

    STAGE_DIR="$(mktemp -d)"
    cp -a -- "$APP_SOURCE/contents" "$STAGE_DIR/contents"
    CONTENTS_INSTALL="$STAGE_DIR/contents"
    sed -i "s|\$HOME/.cargo/bin/soniccore|$CARGO_BIN_DIR/soniccore|" \
        "$CONTENTS_INSTALL/ui/SonicCore.qml" \
        "$CONTENTS_INSTALL/ui/configBattery.qml" \
        "$CONTENTS_INSTALL/ui/configOutput.qml"
    SERVICE_INSTALL="$STAGE_DIR/$(basename -- "$SERVICE_SOURCE")"
    EQUALIZER_SERVICE_INSTALL="$STAGE_DIR/$(basename -- "$EQUALIZER_SERVICE_SOURCE")"
    SPATIAL_SERVICE_INSTALL="$STAGE_DIR/$(basename -- "$SPATIAL_SERVICE_SOURCE")"
    sed "s|%h/.cargo/bin/|$CARGO_BIN_DIR/|g" "$SERVICE_SOURCE" > "$SERVICE_INSTALL"
    sed "s|%h/.cargo/bin/|$CARGO_BIN_DIR/|g" "$EQUALIZER_SERVICE_SOURCE" > "$EQUALIZER_SERVICE_INSTALL"
    sed "s|%h/.cargo/bin/|$CARGO_BIN_DIR/|g" "$SPATIAL_SERVICE_SOURCE" > "$SPATIAL_SERVICE_INSTALL"
    local staged
    for staged in "$CONTENTS_INSTALL/ui/SonicCore.qml" \
        "$CONTENTS_INSTALL/ui/configBattery.qml" \
        "$CONTENTS_INSTALL/ui/configOutput.qml" "$SERVICE_INSTALL" \
        "$EQUALIZER_SERVICE_INSTALL" "$SPATIAL_SERVICE_INSTALL"; do
        grep -Fq "$CARGO_BIN_DIR/soniccore" "$staged" \
            || fail "não foi possível apontar $staged para $CARGO_BIN_DIR"
        ! grep -Fq '.cargo/bin' "$staged" || [[ "$CARGO_BIN_DIR" == *.cargo/bin* ]] \
            || fail "$staged ainda aponta para ~/.cargo/bin"
    done
}

if [[ "${1:-}" == "--flatpak" ]]; then
    FLATPAK_MODE=1
    shift
fi

if [[ "${1:-}" == "--check" ]]; then
    check_prerequisites
    [[ "$FLATPAK_MODE" -eq 1 ]] || check_qml_modules
    stage_packaging
    "$ACP_OVERLAY" --check
    printf 'Pré-requisitos do JamBaLinux Sonic Core verificados. Nenhuma alteração foi feita.\n'
    exit 0
fi

[[ $# -eq 0 ]] || fail "uso: tools/install-user.sh [--flatpak] [--check]"
check_prerequisites
[[ "$FLATPAK_MODE" -eq 1 ]] || check_qml_modules
stage_packaging

# The app's launcher: a small Qt program that opens the interface with its
# helpers (commands, D-Bus, media, volume) built in, on any desktop. With
# --flatpak the app is built by tools/build-flatpak.sh instead.
if [[ "$FLATPAK_MODE" -eq 0 ]]; then
    BUILD_DIR="$(mktemp -d)"
    (
        cd -- "$BUILD_DIR"
        "$QMAKE" "$APP_SOURCE/launcher/soniccore-app.pro" >/dev/null
        make -j"$(nproc 2>/dev/null || echo 2)" >/dev/null
    ) || fail "não foi possível compilar o app (soniccore-app); veja as mensagens acima"
    [[ -x "$BUILD_DIR/soniccore-app" ]] || fail "a compilação não gerou soniccore-app"
fi

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
if [[ "$FLATPAK_MODE" -eq 1 ]]; then
    # The Flatpak brings its own menu entry; at login it starts in the tray.
    rm -f -- "$LAUNCHER_TARGET"
    mkdir -p -- "$(dirname -- "$AUTOSTART_TARGET")"
    sed "s|^Exec=.*|Exec=flatpak run $FLATPAK_APP_ID --tray|" "$AUTOSTART_SOURCE" > "$AUTOSTART_TARGET"
else
    install -Dm644 "$LAUNCHER_INSTALL" "$LAUNCHER_TARGET"
    install -Dm644 "$AUTOSTART_SOURCE" "$AUTOSTART_TARGET"
fi
install -Dm644 "$ICON_SOURCE" "$ICON_TARGET"
# An icon cache in the user's hicolor folder that predates the icon hides
# it (Qt and KWin trust the cache and show a blank icon). Refresh it, or set
# it aside when it cannot be rebuilt; without it the folder is read directly.
ICON_CACHE="$(dirname -- "$(dirname -- "$(dirname -- "$ICON_TARGET")")")/icon-theme.cache"
if [[ -f "$ICON_CACHE" ]] && ! LC_ALL=C grep -aqF 'scalable/apps' "$ICON_CACHE"; then
    if ! gtk-update-icon-cache -f -q -t "$(dirname -- "$ICON_CACHE")" >/dev/null 2>&1 \
        || ! LC_ALL=C grep -aqF 'scalable/apps' "$ICON_CACHE"; then
        mv -f -- "$ICON_CACHE" "$ICON_CACHE.old"
    fi
fi
# The app carries its own copy of the interface (QML, shaders, images) and
# both sets of controls (Plasma's, used on Plasma, and Qt's, used elsewhere).
rm -rf -- "$APP_TARGET"
if [[ "$FLATPAK_MODE" -eq 0 ]]; then
    install -Dm755 "$BUILD_DIR/soniccore-app" "$APP_TARGET/soniccore-app"
    install -Dm644 "$ICON_SOURCE" "$APP_TARGET/jambalinux-soniccore.svg"
    install -Dm644 "$APP_SOURCE/main.qml" "$APP_TARGET/main.qml"
    cp -r -- "$APP_SOURCE/icons" "$APP_TARGET/icons"
    cp -r -- "$APP_SOURCE/controls" "$APP_TARGET/controls"
    cp -r -- "$CONTENTS_INSTALL" "$APP_TARGET/contents"
fi
# On Plasma, KWin remembers the window's place, size and monitor (Wayland
# apps cannot do it themselves). Other desktops place the window themselves.
if [[ "$FLATPAK_MODE" -eq 0 ]] && command -v kwriteconfig6 >/dev/null 2>&1; then
    "$SCRIPT_DIR/kwin-window-rule.sh" --install || printf 'aviso: não foi possível criar a regra de janela do KWin.\n' >&2
fi
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

if [[ "$FLATPAK_MODE" -eq 1 ]]; then
    "$SCRIPT_DIR/build-flatpak.sh" || fail "o núcleo foi instalado, mas não foi possível compilar o app em Flatpak; rode tools/build-flatpak.sh de novo"
fi

printf '\nJamBaLinux Sonic Core instalado para o usuário atual.\n'
printf 'O app abre pelo menu (JamBaLinux Sonic Core) e inicia na bandeja ao entrar na sessão.\n'
printf 'Se ele já estava aberto, feche-o (Sair, na bandeja) e abra de novo para usar esta versão.\n'
printf 'A regra udev privilegiada também não foi alterada. Consulte o README se houver erro de permissão.\n'
