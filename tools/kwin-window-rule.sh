#!/usr/bin/env bash
# The SonicCore window rule for KWin: remember where the app window was, how
# big, on which monitor and whether maximized, across closing and rebooting.
# Wayland does not let an app place its own window, so KWin has to.
#   --install    add or refresh the rule (only our own group is written)
#   --uninstall  remove it
# Without kwriteconfig6/kreadconfig6 (not KDE) it does nothing.
set -euo pipefail

readonly RULE_ID="jambalinux-soniccore-window"
# Same base as the other installed files, so a test HOME never touches the
# real desktop.
readonly FILE="$HOME/.config/kwinrulesrc"

command -v kwriteconfig6 >/dev/null 2>&1 && command -v kreadconfig6 >/dev/null 2>&1 || exit 0

# KWin 6 keeps the rule order in `Order`; older files use `rules`/`count`.
rules_list() {
    local order
    order="$(kreadconfig6 --file "$FILE" --group General --key Order)"
    [[ -n "$order" ]] || order="$(kreadconfig6 --file "$FILE" --group General --key rules)"
    printf '%s' "$order"
}

set_rules_list() {
    local list="$1" count=0
    [[ -n "$list" ]] && count=$(tr ',' '\n' <<<"$list" | grep -c .)
    kwriteconfig6 --file "$FILE" --group General --key Order "$list"
    kwriteconfig6 --file "$FILE" --group General --key rules "$list"
    kwriteconfig6 --file "$FILE" --group General --key count "$count"
}

without_ours() {
    tr ',' '\n' <<<"$(rules_list)" | grep -v -x -e "$RULE_ID" -e '' | paste -sd, - || true
}

reconfigure() {
    qdbus6 org.kde.KWin /KWin reconfigure >/dev/null 2>&1 \
        || qdbus org.kde.KWin /KWin reconfigure >/dev/null 2>&1 || true
}

write() {
    kwriteconfig6 --file "$FILE" --group "$RULE_ID" --key "$1" "$2"
}

case "${1:-}" in
    --install)
        write Description "JamBaLinux SonicCore"
        # The qml6 runtime's generic class plus the exact window title.
        write wmclass "org.qt-project.qml6"
        write wmclassmatch 1
        write wmclasscomplete false
        write title "JamBaLinux SonicCore"
        write titlematch 1
        write types 1
        # 4 = Remember: KWin keeps the last value the user left.
        write positionrule 4
        write sizerule 4
        write screenrule 4
        write maximizehorizrule 4
        write maximizevertrule 4
        others="$(without_ours)"
        set_rules_list "${others:+$others,}$RULE_ID"
        reconfigure
        ;;
    --uninstall)
        kwriteconfig6 --file "$FILE" --group "$RULE_ID" --key Description --delete
        for key in wmclass wmclassmatch wmclasscomplete title titlematch types positionrule \
            sizerule screenrule maximizehorizrule maximizevertrule position size screen \
            maximizehoriz maximizevert; do
            kwriteconfig6 --file "$FILE" --group "$RULE_ID" --key "$key" --delete
        done
        set_rules_list "$(without_ours)"
        reconfigure
        ;;
    *)
        printf 'uso: tools/kwin-window-rule.sh --install | --uninstall\n' >&2
        exit 2
        ;;
esac
