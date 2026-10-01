#!/usr/bin/env bash
set -euo pipefail

readonly PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
readonly OVERLAY="$PROJECT_DIR/tools/acp-overlay.sh"
readonly PACKAGING="$PROJECT_DIR/packaging"
readonly PROFILE_SET="$PACKAGING/alsa-card-profile/profile-sets/jambalinux-quantum810.conf"
readonly TEMP_HOME="$(mktemp -d)"
# A space in the config path catches word-splitting and manifest parsing bugs.
readonly CONFIG="$TEMP_HOME/config dir"

cleanup() {
    rm -rf -- "$TEMP_HOME"
}
trap cleanup EXIT

run_overlay() {
    HOME="$TEMP_HOME" XDG_CONFIG_HOME="$CONFIG" "$OVERLAY" "$@"
}

# Prints the value of KEY inside [SECTION] of an ACP ini file.
ini_value() {
    local file="$1" section="$2" key="$3"
    awk -v section="[$section]" -v key="$key" '
        /^\[/ { inside = ($0 == section); next }
        inside && $1 == key && $2 == "=" { sub(/^[^=]*= */, ""); print; exit }
    ' "$file"
}

# `! cmd` never trips `set -e`, so negative assertions must fail explicitly.
refute() {
    if "$@"; then
        printf 'assertion refuted: %s\n' "$*" >&2
        exit 1
    fi
}

path_file() {
    printf '%s/alsa-card-profile/paths/%s.conf' "$PACKAGING" "$1"
}

test_game_controls_pcm0_and_chat_controls_pcm1() {
    local game chat chat_mono
    game="$(ini_value "$PROFILE_SET" 'Mapping stereo-game-output' paths-output)"
    chat="$(ini_value "$PROFILE_SET" 'Mapping stereo-chat-output' paths-output)"
    chat_mono="$(ini_value "$PROFILE_SET" 'Mapping mono-chat-output' paths-output)"

    [[ "$(ini_value "$PROFILE_SET" 'Mapping stereo-game-output' device-strings)" == 'hw:%f,0,0' ]]
    [[ "$(ini_value "$PROFILE_SET" 'Mapping stereo-chat-output' device-strings)" == 'hw:%f,1,0' ]]
    [[ "$(ini_value "$PROFILE_SET" 'Mapping mono-chat-output' device-strings)" == 'hw:%f,1,0' ]]

    # Game (PCM 0) owns the "PCM" element and never PCM,1.
    grep -qx '\[Element PCM\]' "$(path_file "$game")"
    refute grep -q '^\[Element PCM,1\]' "$(path_file "$game")"
    # Both chat outputs (PCM 1) own "PCM,1" and never the Game element.
    for path in "$chat" "$chat_mono"; do
        grep -qx '\[Element PCM,1\]' "$(path_file "$path")"
        refute grep -qx '\[Element PCM\]' "$(path_file "$path")"
    done
    [[ "$game" != 'usb-gaming-headset-output-stereo' ]]
}

test_mapping_and_profile_names_match_upstream() {
    local sections expected
    sections="$(grep -E '^\[(Mapping|Profile) ' "$PROFILE_SET" | LC_ALL=C sort)"
    expected="$(printf '%s\n' \
        '[Mapping mono-chat-input]' \
        '[Mapping mono-chat-output]' \
        '[Mapping stereo-chat-output]' \
        '[Mapping stereo-game-output]' \
        '[Profile output:mono-chat+output:stereo-game+input:mono-chat]' \
        '[Profile output:stereo-game+output:stereo-chat+input:mono-chat]' | LC_ALL=C sort)"
    [[ "$sections" == "$expected" ]]
}

test_wireplumber_rule_targets_only_the_quantum810_card() {
    local rule="$PACKAGING/wireplumber/51-jambalinux-quantum810-acp.conf"
    grep -Fq 'device.name = "~alsa_card.usb-Harman_International_Inc_JBL_Quantum810_Wireless-.*"' "$rule"
    grep -Fq 'device.profile-set = "jambalinux-quantum810.conf"' "$rule"
    [[ "$(grep -c 'device.name' "$rule")" -eq 1 ]]
}

test_overlay_never_uses_sudo_or_system_paths() {
    local code
    # Comments may explain what the script avoids; only executable lines count.
    code="$(grep -vE '^[[:space:]]*#' "$OVERLAY")"
    refute grep -qE '(^|[^[:alnum:]_])sudo([^[:alnum:]_]|$)' <<< "$code"
    refute grep -qE '(^|[ "=])/(usr|etc)/' <<< "$code"
}

test_check_changes_nothing() {
    local output
    output="$(run_overlay --check)"
    [[ "$output" == *"ausente"*"jambalinux-quantum810.conf"* ]]
    [[ ! -e "$CONFIG" ]]
}

test_install_is_idempotent_and_respects_xdg_config_home() {
    local acp="$CONFIG/alsa-card-profile/mixer"
    run_overlay --install >/dev/null
    run_overlay --install >/dev/null
    cmp -s "$PROFILE_SET" "$acp/profile-sets/jambalinux-quantum810.conf"
    for path in game chat chat-mono; do
        cmp -s "$(path_file "jambalinux-quantum810-$path")" "$acp/paths/jambalinux-quantum810-$path.conf"
    done
    [[ -f "$CONFIG/wireplumber/wireplumber.conf.d/51-jambalinux-quantum810-acp.conf" ]]
    [[ ! -e "$TEMP_HOME/.config" ]]
    [[ "$(run_overlay --check)" != *"ausente"* ]]
}

test_uninstall_keeps_foreign_and_modified_files() {
    local acp="$CONFIG/alsa-card-profile/mixer"
    local foreign="$acp/paths/someone-else.conf"
    local edited="$acp/paths/jambalinux-quantum810-chat.conf"
    local status

    run_overlay --install >/dev/null
    printf '[General]\n' > "$foreign"
    printf '# editado\n' >> "$edited"

    set +e
    run_overlay --uninstall >/dev/null 2>&1
    status=$?
    set -e

    [[ "$status" -ne 0 ]]
    [[ -f "$foreign" ]]
    [[ -f "$edited" ]]
    [[ ! -e "$acp/profile-sets/jambalinux-quantum810.conf" ]]
    [[ ! -e "$acp/paths/jambalinux-quantum810-game.conf" ]]
    [[ ! -e "$CONFIG/wireplumber/wireplumber.conf.d/51-jambalinux-quantum810-acp.conf" ]]

    rm -f -- "$edited"
    run_overlay --uninstall >/dev/null
}

test_install_refuses_to_overwrite_an_edited_file() {
    local edited="$CONFIG/alsa-card-profile/mixer/paths/jambalinux-quantum810-game.conf"
    local status

    run_overlay --install >/dev/null
    printf '# ajuste local\n' >> "$edited"

    set +e
    run_overlay --install >/dev/null 2>&1
    status=$?
    set -e

    [[ "$status" -ne 0 ]]
    grep -Fqx '# ajuste local' "$edited"
    [[ "$(run_overlay --check)" == *"modificado"*"jambalinux-quantum810-game.conf"* ]]

    rm -f -- "$edited"
    run_overlay --uninstall >/dev/null
}

test_install_upgrades_a_file_it_installed_earlier() {
    local config="$CONFIG"
    local game="$config/alsa-card-profile/mixer/paths/jambalinux-quantum810-game.conf"
    local manifest="$config/jambalinux-soniccore/acp-overlay.sha256"
    local old_hash new_hash

    run_overlay --install >/dev/null
    # Pretend an older JamBaLinux release installed different content.
    old_hash="$(sha256sum -- "$game" | cut -d' ' -f1)"
    printf '# versão anterior\n' >> "$game"
    new_hash="$(sha256sum -- "$game" | cut -d' ' -f1)"
    sed -i "s/^$old_hash /$new_hash /" "$manifest"
    [[ "$(run_overlay --check)" == *"desatualizado"* ]]

    run_overlay --install >/dev/null
    cmp -s "$(path_file jambalinux-quantum810-game)" "$game"

    run_overlay --uninstall >/dev/null
    [[ ! -e "$manifest" ]]
}

test_user_scripts_install_and_remove_the_overlay() {
    grep -Fq '"$ACP_OVERLAY" --install' "$PROJECT_DIR/tools/install-user.sh"
    grep -Fq '"$ACP_OVERLAY" --check' "$PROJECT_DIR/tools/install-user.sh"
    grep -Fq '"$ACP_OVERLAY" --uninstall' "$PROJECT_DIR/tools/uninstall-user.sh"
    grep -Fq '"$ACP_OVERLAY" --check' "$PROJECT_DIR/tools/uninstall-user.sh"
}

test_game_controls_pcm0_and_chat_controls_pcm1
test_mapping_and_profile_names_match_upstream
test_wireplumber_rule_targets_only_the_quantum810_card
test_overlay_never_uses_sudo_or_system_paths
test_check_changes_nothing
test_install_is_idempotent_and_respects_xdg_config_home
test_uninstall_keeps_foreign_and_modified_files
test_install_refuses_to_overwrite_an_edited_file
test_install_upgrades_a_file_it_installed_earlier
test_user_scripts_install_and_remove_the_overlay

printf '%s\n' 'acp-overlay tests passed'
