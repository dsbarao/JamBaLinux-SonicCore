#!/usr/bin/env bash
# Overlay ACP do usuário para o JBL Quantum 810 Wireless: o sink Game passa a
# controlar o PCM 0 e o Chat o PCM 1. Só escreve em XDG_CONFIG_HOME (padrão
# ~/.config); nunca usa sudo nem toca /usr, regras udev, HID ou VID/PID.
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
readonly SOURCE_DIR="$PROJECT_DIR/packaging"
readonly CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}"
# sha256 of each file as JamBaLinux last installed it. A target that matches
# its recorded hash (or the current source) is ours to replace or remove;
# anything else was edited outside JamBaLinux and is left alone.
readonly MANIFEST="$CONFIG_DIR/jambalinux-soniccore/acp-overlay.sha256"

# Pares "origem no repositório|destino no usuário". Só estes arquivos pertencem
# ao overlay; o desinstalador nunca remove outra coisa.
overlay_files() {
    local acp="$CONFIG_DIR/alsa-card-profile/mixer"
    printf '%s\n' \
        "alsa-card-profile/profile-sets/jambalinux-quantum810.conf|$acp/profile-sets/jambalinux-quantum810.conf" \
        "alsa-card-profile/paths/jambalinux-quantum810-game.conf|$acp/paths/jambalinux-quantum810-game.conf" \
        "alsa-card-profile/paths/jambalinux-quantum810-chat.conf|$acp/paths/jambalinux-quantum810-chat.conf" \
        "alsa-card-profile/paths/jambalinux-quantum810-chat-mono.conf|$acp/paths/jambalinux-quantum810-chat-mono.conf" \
        "wireplumber/51-jambalinux-quantum810-acp.conf|$CONFIG_DIR/wireplumber/wireplumber.conf.d/51-jambalinux-quantum810-acp.conf"
}

fail() {
    printf 'erro: %s\n' "$*" >&2
    exit 1
}

recorded_hash() {
    local hash path
    [[ -f "$MANIFEST" ]] || return 0
    # `read` keeps the rest of the line, spaces included, as the path.
    while read -r hash path; do
        if [[ "$path" == "$1" ]]; then
            printf '%s\n' "$hash"
            return 0
        fi
    done < "$MANIFEST"
}

# Owned: absent, identical to the current source, or unchanged since we
# installed it (an older JamBaLinux version being upgraded).
is_owned() {
    local source="$1" target="$2" recorded
    [[ -e "$target" ]] || return 0
    cmp -s -- "$SOURCE_DIR/$source" "$target" && return 0
    recorded="$(recorded_hash "$target")"
    [[ -n "$recorded" && "$recorded" == "$(sha256sum -- "$target" | cut -d' ' -f1)" ]]
}

activation_note() {
    printf '%s\n' 'Para aplicar, reconecte o dongle ou rode: systemctl --user restart wireplumber'
}

check_overlay() {
    local source target
    printf '%s\n' 'Overlay ACP do Quantum 810 (Game -> PCM 0, Chat -> PCM 1):'
    while IFS='|' read -r source target; do
        [[ -f "$SOURCE_DIR/$source" ]] || fail "arquivo do overlay não encontrado: packaging/$source"
        if [[ ! -e "$target" ]]; then
            printf '  ausente     %s\n' "$target"
        elif cmp -s -- "$SOURCE_DIR/$source" "$target"; then
            printf '  instalado   %s\n' "$target"
        elif is_owned "$source" "$target"; then
            printf '  desatualizado %s\n' "$target"
        else
            printf '  modificado  %s\n' "$target"
        fi
    done < <(overlay_files)
}

install_overlay() {
    local source target manifest_tmp
    while IFS='|' read -r source target; do
        [[ -f "$SOURCE_DIR/$source" ]] || fail "arquivo do overlay não encontrado: packaging/$source"
        is_owned "$source" "$target" \
            || fail "$target foi modificado fora do JamBaLinux; revise ou remova o arquivo e rode de novo"
    done < <(overlay_files)
    mkdir -p -- "$(dirname -- "$MANIFEST")"
    manifest_tmp="$(mktemp -- "$MANIFEST.XXXXXX")"
    while IFS='|' read -r source target; do
        install -Dm644 -- "$SOURCE_DIR/$source" "$target"
        sha256sum -- "$target" >> "$manifest_tmp"
    done < <(overlay_files)
    mv -f -- "$manifest_tmp" "$MANIFEST"
    printf '%s\n' 'Overlay ACP do Quantum 810 instalado.'
    activation_note
}

uninstall_overlay() {
    local source target kept=0
    while IFS='|' read -r source target; do
        [[ -e "$target" ]] || continue
        if is_owned "$source" "$target"; then
            rm -f -- "$target"
        else
            # Alterado por outra pessoa ou ferramenta: não é mais só nosso.
            printf 'mantido (modificado fora do JamBaLinux): %s\n' "$target" >&2
            kept=1
        fi
    done < <(overlay_files)
    if [[ "$kept" -eq 0 ]]; then
        rm -f -- "$MANIFEST"
        rmdir -- "$(dirname -- "$MANIFEST")" 2>/dev/null || true
        printf '%s\n' 'Overlay ACP do Quantum 810 removido; o profile-set upstream volta a valer.'
    else
        printf '%s\n' 'Overlay ACP removido em parte; revise os arquivos mantidos acima.'
    fi
    activation_note
    return "$kept"
}

case "${1:-}" in
    --check) [[ $# -eq 1 ]] || fail 'uso: tools/acp-overlay.sh --check | --install | --uninstall'; check_overlay ;;
    --install) [[ $# -eq 1 ]] || fail 'uso: tools/acp-overlay.sh --check | --install | --uninstall'; install_overlay ;;
    --uninstall) [[ $# -eq 1 ]] || fail 'uso: tools/acp-overlay.sh --check | --install | --uninstall'; uninstall_overlay ;;
    *) printf '%s\n' 'uso: tools/acp-overlay.sh --check | --install | --uninstall' >&2; exit 2 ;;
esac
