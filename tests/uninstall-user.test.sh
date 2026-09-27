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
    HOME="$TEMP_HOME" PATH="$TEMP_BIN:$PATH" "$UNINSTALLER" "$@"
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

    if ! installed_units="$(grep -oE 'jambalinux-soniccore(-[a-z]+)?\.service' "$INSTALLER" | sort -u)"; then
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

test_check_changes_nothing
test_install_uninstall_unit_parity
test_confirm_stops_spatial_first_and_removes_units

printf '%s\n' 'uninstall-user tests passed'
