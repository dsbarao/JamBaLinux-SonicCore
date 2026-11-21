#!/usr/bin/env bash
# Draws the app's tray icons: the same battery the Plasma widget shows in the
# panel (outline, fill by level, headphones inside, bolt while charging), one
# SVG per 5% step. A tray icon is a picture, not QML, so the app picks one of
# these files. The SVGs are committed; rerun this after changing the drawing.
#
# Plasma caps tray icons at a 22 px square, so the widget's 34x18 battery is
# drawn at the full width of the slot, keeping its shape.
set -euo pipefail

readonly PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
readonly OUT="$PROJECT_DIR/packaging/app/icons"

# Same colours as SonicCore.qml's batteryColor.
color_for() {
    local percent=$1 charging=$2
    if [[ $charging == 1 ]]; then printf '#22d3ee'
    elif (( percent >= 60 )); then printf '#35c759'
    elif (( percent >= 30 )); then printf '#f5c542'
    else printf '#ff453a'
    fi
}

readonly HEADPHONES='<g fill="none" stroke="#fcfcfc" stroke-width="1.6" stroke-linecap="round"><path d="M10.6 21v-3.4a4.4 4.4 0 0 1 8.8 0V21"/></g><g fill="#fcfcfc"><rect x="9.3" y="18.6" width="3" height="4.6" rx="1"/><rect x="17.7" y="18.6" width="3" height="4.6" rx="1"/></g>'
# Twice the widget's bolt: at tray size the small one was hard to see.
readonly BOLT='<path transform="translate(-28.1 -5) scale(2)" d="M28.6 3.5 23.8 11h3.3l-1.5 5.5 5.3-7.6h-3.3l2-5.4z" fill="#f5c542" stroke="#1b1e20" stroke-width=".8" stroke-linejoin="round"/>'

battery() {
    local color=$1 inner=$2
    printf '<svg xmlns="http://www.w3.org/2000/svg" width="34" height="34" viewBox="0 0 34 34">'
    printf '<rect x="0.75" y="9.75" width="29.5" height="15.5" rx="3" fill="none" stroke="%s" stroke-width="1.5"/>' "$color"
    printf '<rect x="31" y="14" width="2.5" height="7" rx="1" fill="%s"/>' "$color"
    printf '%s</svg>\n' "$inner"
}

mkdir -p -- "$OUT"
rm -f -- "$OUT"/battery-*.svg

for (( percent = 0; percent <= 100; percent += 5 )); do
    for charging in 0 1; do
        color=$(color_for "$percent" "$charging")
        width=$(awk -v p="$percent" 'BEGIN { w = 26 * p / 100; if (w < 2) w = 2; printf "%.1f", w }')
        fill="<rect x=\"2.5\" y=\"11.5\" width=\"$width\" height=\"12\" rx=\"1\" fill=\"$color\" fill-opacity=\".85\"/>"
        bolt=""
        name="battery-$percent"
        if [[ $charging == 1 ]]; then bolt=$BOLT; name+="-charging"; fi
        battery "$color" "$fill$HEADPHONES$bolt" > "$OUT/$name.svg"
    done
done

# Headset off or not answering: grey outline with a dash, like the widget.
# Off but on the USB cable it still charges, so the bolt stays.
readonly DASH='<rect x="10" y="16.5" width="10" height="2" rx="1" fill="#7f8c8d"/>'
battery '#7f8c8d' "$DASH" > "$OUT/battery-unknown.svg"
battery '#7f8c8d' "$DASH$BOLT" > "$OUT/battery-unknown-charging.svg"

printf 'Ícones da bandeja gerados em %s\n' "$OUT"
