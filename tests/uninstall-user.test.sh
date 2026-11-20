#!/usr/bin/env bash
set -euo pipefail

readonly PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
readonly INSTALLER="$PROJECT_DIR/tools/install-user.sh"
readonly UNINSTALLER="$PROJECT_DIR/tools/uninstall-user.sh"
readonly TEMP_HOME="$(mktemp -d)"
readonly TEMP_BIN="$TEMP_HOME/bin"
readonly SYSTEMCTL_LOG="$TEMP_HOME/systemctl.log"
readonly UNIT_DIR="$TEMP_HOME/.config/systemd/user"
readonly SERVICE_UNIT="$UNIT_DIR/jambalinux-soniccore.service"
readonly EQUALIZER_UNIT="$UNIT_DIR/jambalinux-soniccore-equalizer.service"
readonly SPATIAL_UNIT="$UNIT_DIR/jambalinux-soniccore-spatial.service"
export SYSTEMCTL_LOG

cleanup() {
    rm -rf -- "$TEMP_HOME"
}
trap cleanup EXIT

mkdir -p -- "$TEMP_BIN" "$UNIT_DIR"
touch -- "$SERVICE_UNIT" "$EQUALIZER_UNIT" "$SPATIAL_UNIT" "$SYSTEMCTL_LOG"

cat > "$TEMP_BIN/systemctl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$SYSTEMCTL_LOG"
EOF
chmod +x -- "$TEMP_BIN/systemctl"

run_uninstaller() {
    HOME="$TEMP_HOME" CARGO_HOME="$TEMP_HOME/.cargo" PATH="$TEMP_BIN:$PATH" "$UNINSTALLER" "$@"
}

run_uninstaller_with_cargo_home() {
    local cargo_home="$1"
    shift
    HOME="$TEMP_HOME" CARGO_HOME="$cargo_home" PATH="$TEMP_BIN:$PATH" "$UNINSTALLER" "$@"
}

test_install_check_reports_each_required_audio_client() {
    local command
    local check_bin
    local missing
    local output
    local status

    for missing in pactl pw-link pw-cli pw-dump; do
        check_bin="$TEMP_HOME/check-bin-$missing"
        mkdir -p -- "$check_bin"
        for command in cargo kpackagetool6 pactl pipewire pw-link pw-cli pw-dump systemctl; do
            [[ "$command" == "$missing" ]] && continue
            printf '#!/bin/sh\nexit 0\n' > "$check_bin/$command"
            chmod +x -- "$check_bin/$command"
        done
        ln -s -- "$(command -v dirname)" "$check_bin/dirname"

        set +e
        output="$(HOME="$TEMP_HOME" PATH="$check_bin" /bin/bash "$INSTALLER" --check 2>&1)"
        status=$?
        set -e
        [[ "$status" -ne 0 ]]
        [[ "$output" == *"comando obrigatório não encontrado: $missing" ]]
    done
}

test_cargo_home_is_used_for_uninstaller_binary() {
    local cargo_home="$TEMP_HOME/custom-cargo-home"
    local check_output

    check_output="$(run_uninstaller_with_cargo_home "$cargo_home" --check)"
    [[ "$check_output" == *"$cargo_home/bin/soniccore"* ]]
    grep -Fq 'readonly CARGO_BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"' "$INSTALLER"
    grep -Fq 'readonly CARGO_BINARY="${CARGO_HOME:-$HOME/.cargo}/bin/soniccore"' "$UNINSTALLER"
}

test_check_changes_nothing() {
    local check_output
    check_output="$(run_uninstaller --check)"

    [[ "$check_output" == *"$SPATIAL_UNIT"* ]]
    [[ -f "$SERVICE_UNIT" ]]
    [[ -f "$EQUALIZER_UNIT" ]]
    [[ -f "$SPATIAL_UNIT" ]]
    [[ ! -s "$SYSTEMCTL_LOG" ]]
}

test_install_uninstall_unit_parity() {
    local installed_units
    local unit
    local expected_units

    if ! installed_units="$(grep -oE 'jambalinux-soniccore(-[a-z]+)?\.service' "$INSTALLER" | LC_ALL=C sort -u)"; then
        printf '%s\n' 'failed to find systemd units in the installer' >&2
        return 1
    fi

    expected_units=$'jambalinux-soniccore-equalizer.service\njambalinux-soniccore-spatial.service\njambalinux-soniccore.service'
    [[ -n "$installed_units" ]]
    [[ "$installed_units" == "$expected_units" ]]

    while IFS= read -r unit; do
        grep -Fq "\$HOME/.config/systemd/user/$unit" "$UNINSTALLER"
    done <<< "$installed_units"
}

test_confirm_stops_spatial_first_and_removes_units() {
    local spatial_line
    local equalizer_line

    run_uninstaller --confirm >/dev/null

    spatial_line="$(grep -nFx -- '--user disable --now jambalinux-soniccore-spatial.service' "$SYSTEMCTL_LOG" | cut -d: -f1)"
    equalizer_line="$(grep -nFx -- '--user disable --now jambalinux-soniccore-equalizer.service' "$SYSTEMCTL_LOG" | cut -d: -f1)"
    [[ -n "$spatial_line" ]]
    [[ -n "$equalizer_line" ]]
    [[ "$spatial_line" -lt "$equalizer_line" ]]
    [[ ! -e "$SERVICE_UNIT" ]]
    [[ ! -e "$EQUALIZER_UNIT" ]]
    [[ ! -e "$SPATIAL_UNIT" ]]
}

test_partial_overlay_removal_is_reported_and_fails() {
    local edited="$TEMP_HOME/.config/alsa-card-profile/mixer/paths/jambalinux-quantum810-chat.conf"
    local output status

    HOME="$TEMP_HOME" "$PROJECT_DIR/tools/acp-overlay.sh" --install >/dev/null
    printf '# editado\n' >> "$edited"

    set +e
    output="$(run_uninstaller --confirm 2>&1)"
    status=$?
    set -e

    [[ "$status" -ne 0 ]]
    [[ "$output" == *"Remoção parcial"* ]]
    [[ "$output" != *"foram removidos."* ]]
    [[ -f "$edited" ]]
    rm -f -- "$edited"
}

make_install_stubs() {
    local stub_bin="$1"
    mkdir -p -- "$stub_bin"
    # cargo install: create the binaries where CARGO_HOME says.
    cat > "$stub_bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf 'cargo %s\n' "$*" >> "$INSTALL_LOG"
mkdir -p -- "$CARGO_HOME/bin"
printf '#!/bin/sh\n' > "$CARGO_HOME/bin/soniccore"
chmod +x -- "$CARGO_HOME/bin/soniccore"
STUB
    # kpackagetool6 --type Plasma/Applet --install|--upgrade DIR
    cat > "$stub_bin/kpackagetool6" <<'STUB'
#!/usr/bin/env bash
target="$HOME/.local/share/plasma/plasmoids"
mkdir -p -- "$target"
rm -rf -- "$target/org.jambalinux.soniccore"
cp -a -- "${@: -1}" "$target/org.jambalinux.soniccore"
STUB
    local command
    for command in pactl pipewire pw-link pw-cli pw-dump; do
        printf '#!/bin/sh\nexit 0\n' > "$stub_bin/$command"
    done
    cp -- "$TEMP_BIN/systemctl" "$stub_bin/systemctl"
    chmod +x -- "$stub_bin"/*
}

test_custom_cargo_home_is_written_into_installed_files() {
    local home="$TEMP_HOME/custom-install-home"
    local stub_bin="$home/stub-bin"
    local cargo_home="$home/opt/cargo"
    mkdir -p -- "$home"
    make_install_stubs "$stub_bin"

    HOME="$home" CARGO_HOME="$cargo_home" INSTALL_LOG="$home/install.log" \
        PATH="$stub_bin:$PATH" /bin/bash "$INSTALLER" >/dev/null

    local unit
    for unit in jambalinux-soniccore jambalinux-soniccore-equalizer jambalinux-soniccore-spatial; do
        grep -Fq "ExecStart=$cargo_home/bin/soniccore" "$home/.config/systemd/user/$unit.service"
        ! grep -Fq '.cargo/bin' "$home/.config/systemd/user/$unit.service"
    done
    grep -Fqx "Exec=$cargo_home/bin/soniccore meter --dbus" \
        "$home/.local/share/dbus-1/services/org.jambalinux.soniccore.Meter.service"
    grep -Fq "exec $cargo_home/bin/soniccore show" \
        "$home/.local/share/applications/jambalinux-soniccore.desktop"
    grep -Fq "\"$cargo_home/bin/soniccore\"" \
        "$home/.local/share/plasma/plasmoids/org.jambalinux.soniccore/contents/ui/main.qml"
    # The repository copies stay untouched.
    grep -Fq 'ExecStart=%h/.cargo/bin/soniccore' "$PROJECT_DIR/packaging/systemd/jambalinux-soniccore.service"
}

test_default_cargo_home_installs_packaging_verbatim() {
    local home="$TEMP_HOME/default-install-home"
    local stub_bin="$home/stub-bin"
    mkdir -p -- "$home"
    make_install_stubs "$stub_bin"

    HOME="$home" CARGO_HOME="$home/.cargo" INSTALL_LOG="$home/install.log" \
        PATH="$stub_bin:$PATH" /bin/bash "$INSTALLER" >/dev/null

    cmp -s -- "$PROJECT_DIR/packaging/systemd/jambalinux-soniccore.service" \
        "$home/.config/systemd/user/jambalinux-soniccore.service"
    grep -Fqx "Exec=$home/.cargo/bin/soniccore meter --dbus" \
        "$home/.local/share/dbus-1/services/org.jambalinux.soniccore.Meter.service"
    cmp -s -- "$PROJECT_DIR/packaging/plasma/org.jambalinux.soniccore/contents/ui/main.qml" \
        "$home/.local/share/plasma/plasmoids/org.jambalinux.soniccore/contents/ui/main.qml"
}

test_edited_overlay_stops_install_before_any_change() {
    local home="$TEMP_HOME/overlay-install-home"
    local stub_bin="$home/stub-bin"
    local edited="$home/.config/alsa-card-profile/mixer/paths/jambalinux-quantum810-game.conf"
    local output status
    mkdir -p -- "$home"
    make_install_stubs "$stub_bin"
    HOME="$home" "$PROJECT_DIR/tools/acp-overlay.sh" --install >/dev/null
    printf '# editado\n' >> "$edited"

    set +e
    output="$(HOME="$home" CARGO_HOME="$home/.cargo" INSTALL_LOG="$home/install.log" \
        PATH="$stub_bin:$PATH" /bin/bash "$INSTALLER" 2>&1)"
    status=$?
    set -e

    [[ "$status" -ne 0 ]]
    [[ "$output" == *"modificado fora do JamBaLinux"* ]]
    [[ ! -e "$home/install.log" ]]
    [[ ! -e "$home/.config/systemd/user/jambalinux-soniccore.service" ]]
}

test_unsafe_cargo_home_is_refused() {
    local home="$TEMP_HOME/unsafe-install-home"
    local stub_bin="$home/stub-bin"
    local output status
    mkdir -p -- "$home"
    make_install_stubs "$stub_bin"

    set +e
    output="$(HOME="$home" CARGO_HOME="$home/my cargo" INSTALL_LOG="$home/install.log" \
        PATH="$stub_bin:$PATH" /bin/bash "$INSTALLER" 2>&1)"
    status=$?
    set -e

    [[ "$status" -ne 0 ]]
    [[ "$output" == *"não é suportado"* ]]
    [[ ! -e "$home/install.log" ]]
}

test_check_changes_nothing
test_install_check_reports_each_required_audio_client
test_custom_cargo_home_is_written_into_installed_files
test_default_cargo_home_installs_packaging_verbatim
test_edited_overlay_stops_install_before_any_change
test_unsafe_cargo_home_is_refused
test_cargo_home_is_used_for_uninstaller_binary
test_install_uninstall_unit_parity
test_confirm_stops_spatial_first_and_removes_units
test_partial_overlay_removal_is_reported_and_fails

printf '%s\n' 'uninstall-user tests passed'
