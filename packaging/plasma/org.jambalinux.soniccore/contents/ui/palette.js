.pragma library

// The Vivo theme's palette: the headset lights' own colors, in order (logo
// segments, then ring segments), without black, repeats or invalid values.
// An empty result means "use the system accent".
function build(enabled, logo, ring, maximum) {
    if (enabled === false) return []
    const limit = maximum || 5
    const seen = {}
    const result = []
    const zones = [logo, ring]
    for (let zone = 0; zone < zones.length; ++zone) {
        const value = zones[zone]
        const list = Array.isArray(value) ? value : (value === null || value === undefined ? [] : [value])
        for (let index = 0; index < list.length; ++index) {
            const hex = normalize(list[index])
            if (!hex || hex === "#000000" || seen[hex]) continue
            seen[hex] = true
            result.push(hex)
            if (result.length >= limit) return result
        }
    }
    return result
}

function normalize(value) {
    if (typeof value !== "string") return ""
    let hex = value.trim().toLowerCase()
    if (!hex.startsWith("#")) hex = "#" + hex
    return /^#[0-9a-f]{6}$/.test(hex) ? hex : ""
}

function channels(hex) {
    return [parseInt(hex.substr(1, 2), 16) / 255,
            parseInt(hex.substr(3, 2), 16) / 255,
            parseInt(hex.substr(5, 2), 16) / 255]
}

// The color at position t (0..1) along the palette, blending neighbours
// smoothly; `fallback` when the palette is empty.
function at(palette, t, fallback) {
    if (!palette || palette.length === 0) return fallback
    if (palette.length === 1) {
        const only = channels(palette[0])
        return Qt.rgba(only[0], only[1], only[2], 1)
    }
    const position = Math.max(0, Math.min(1, t)) * (palette.length - 1)
    const lower = Math.min(palette.length - 2, Math.floor(position))
    const amount = position - lower
    const first = channels(palette[lower])
    const second = channels(palette[lower + 1])
    return Qt.rgba(first[0] + (second[0] - first[0]) * amount,
                   first[1] + (second[1] - first[1]) * amount,
                   first[2] + (second[2] - first[2]) * amount, 1)
}

// The same hue at full brightness: OpenRGB effects often send the computer's
// colors dimmed (a fade, an idle visualizer), which as a theme reads as
// black. Colors too dark to have a hue become black (left out of a palette).
function fullBrightness(value) {
    const hex = normalize(value)
    if (!hex) return ""
    const rgb = channels(hex)
    const peak = Math.max(rgb[0], rgb[1], rgb[2])
    if (peak < 4 / 255) return "#000000"
    const part = channel => Math.round(channel / peak * 255).toString(16).padStart(2, "0")
    return "#" + part(rgb[0]) + part(rgb[1]) + part(rgb[2])
}
