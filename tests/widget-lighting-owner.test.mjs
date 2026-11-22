import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/app/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

const controlsMatch = widgetSource.match(
    /readonly property bool openRgbControlsLighting: ([\s\S]*?)\n\s+readonly property bool lightingWritable: (.*)\n/)
assert.notEqual(controlsMatch, null, "SonicCore.qml must expose openRgbControlsLighting and lightingWritable")

function evaluate({ lightingHeld, lightingSource, nowMs, lightingUpdatedAtMs, deviceAvailable = true }) {
    const openRgbControlsLighting = new Function(
        "lightingHeld", "lightingSource", "nowMs", "lightingUpdatedAtMs",
        `return ${controlsMatch[1]}`)(lightingHeld, lightingSource, nowMs, lightingUpdatedAtMs)
    const lightingWritable = new Function("deviceAvailable", "openRgbControlsLighting",
        `return ${controlsMatch[2]}`)(deviceAvailable, openRgbControlsLighting)
    return { openRgbControlsLighting, lightingWritable }
}

test("OpenRGB escrevendo agora trava a seção de luzes", () => {
    const now = 1_000_000
    const state = evaluate({ lightingHeld: false, lightingSource: "openrgb", nowMs: now, lightingUpdatedAtMs: now - 500 })
    assert.equal(state.openRgbControlsLighting, true)
    assert.equal(state.lightingWritable, false)
})

test("OpenRGB parado há mais de 5 s libera a seção", () => {
    const now = 1_000_000
    const state = evaluate({ lightingHeld: false, lightingSource: "openrgb", nowMs: now, lightingUpdatedAtMs: now - 6000 })
    assert.equal(state.lightingWritable, true)
})

test("com as luzes assumidas, a seção fica livre mesmo com o OpenRGB ativo", () => {
    const now = 1_000_000
    const state = evaluate({ lightingHeld: true, lightingSource: "openrgb", nowMs: now, lightingUpdatedAtMs: now })
    assert.equal(state.openRgbControlsLighting, false)
    assert.equal(state.lightingWritable, true)
})

test("escritas do próprio SonicCore nunca travam a seção", () => {
    const now = 1_000_000
    assert.equal(evaluate({ lightingHeld: false, lightingSource: "soniccore", nowMs: now, lightingUpdatedAtMs: now }).lightingWritable, true)
})

test("assumir e devolver usam o comando lighting-hold", () => {
    assert.match(widgetSource, /soniccoreCommand\(\["lighting-hold", held \? "on" : "off"\]\)/)
    assert.match(widgetSource, /text: root\.lightingHeld \? "Devolver ao OpenRGB" : "Assumir controle"/)
    assert.match(widgetSource, /lightingHeld = result\.lighting_held === true/)
})

test("com o fone seguindo o tema das Luzes do PC, só ligar e desligar ficam livres", () => {
    assert.match(widgetSource,
        /readonly property bool headsetFollowsTheme: pcLightsActive && pcLightsStatus\?\.in_charge === true\n\s+&& pcLightsConfig\.headset !== false && pcLightsStatus\?\.theme !== "headset"/)
    assert.match(widgetSource, /readonly property bool lightingColorsWritable: lightingWritable && !headsetFollowsTheme/)
    // On/off keeps the plain rule; colors, effects and the wheel need both.
    assert.match(widgetSource, /enabled: root\.lightingWritable && !root\.controlBusy\n\s+SonicControls\.Button \{ text: "Ligar"/)
    assert.equal((widgetSource.match(/enabled: root\.lightingWritable /g) || []).length, 1)
    assert.match(widgetSource, /text: "Escolher a cor só do fone"[\s\S]{0,120}onClicked: root\.setPcLights\(\["headset", "off"\], \{ "headset": false \}\)/)
})
