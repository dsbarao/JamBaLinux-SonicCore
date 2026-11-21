import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")
const normalizerMatch = widgetSource.match(
    /    function normalizeBatteryPercent\(value\) \{([\s\S]*?)^    \}\n\n    function applyResult/m)
const applyResultMatch = widgetSource.match(
    /    function applyResult\(data\) \{([\s\S]*?)^    \}\n\n    property Component compactRepresentation/m)

assert.notEqual(normalizerMatch, null, "SonicCore.qml must expose the shared battery normalizer")
assert.notEqual(applyResultMatch, null, "SonicCore.qml must apply normalized battery results")
const normalizeBatteryPercent = new Function(
    `function normalizeBatteryPercent(value) {${normalizerMatch[1]}}\nreturn normalizeBatteryPercent;`)()
const applyResult = new Function("normalizeBatteryPercent", "friendlyError", "normalizeColor",
    `return function applyResult(data) { with (this) {${applyResultMatch[1]}} };`)(
    normalizeBatteryPercent, () => "", () => "#33ffcc")

function statusWithBattery(battery) {
    return {
        "exit code": 0,
        stdout: JSON.stringify({
            battery_percent: battery,
            charging: true,
            ambient_mode: "anc",
            microphone: "muted",
            sidetone_level: "medium",
            lighting_enabled: true
        })
    }
}

test("null, undefined, and empty battery values are unknown", () => {
    for (const value of [null, undefined, "", "   "]) {
        assert.equal(normalizeBatteryPercent(value), -1)
    }
})

test("unknown battery does not discard the rest of the status", () => {
    // These are QML properties on the real PlasmoidItem. They must already
    // exist for JavaScript's `with` scope to assign them on this test double.
    const widget = {
        batteryPercent: 0, charging: false, rawFeature: "", ambientMode: "",
        headsetConnected: null, microphoneState: "", sidetoneLevel: "",
        lightingEnabled: null, lightingColor: "", logoColor: "", ringColor: "",
        logoColors: [], ringColors: [], logoEffect: "", ringEffect: "",
        logoSpeed: "", ringSpeed: "", gameChatValue: -1, errorMessage: ""
    }
    applyResult.call(widget, statusWithBattery(null))

    assert.equal(widget.batteryPercent, -1)
    assert.equal(widget.charging, true)
    assert.equal(widget.ambientMode, "anc")
    assert.equal(widget.microphoneState, "muted")
    assert.equal(widget.sidetoneLevel, "medium")
    assert.equal(widget.lightingEnabled, true)
    assert.equal(widget.errorMessage, "")
})

test("numeric battery values remain valid", () => {
    assert.equal(normalizeBatteryPercent(0), 0)
    assert.equal(normalizeBatteryPercent("50"), 50)
    assert.equal(normalizeBatteryPercent(100), 100)
})

test("non-numeric and out-of-range battery values are rejected", () => {
    for (const value of ["unknown", true, -1, 101]) {
        assert.throws(() => normalizeBatteryPercent(value), /percentual inválido/)
    }
})

test("direct and cached statuses share the result normalization and unknown UI", () => {
    // Both the cached and the direct read finish through finishRefresh,
    // which applies the result with the same normalization.
    const finishCalls = widgetSource.match(/root\.finishRefresh\(data\)/g) ?? []
    assert.equal(finishCalls.length, 2)
    assert.match(widgetSource,
        /function finishRefresh\(data\) \{\n\s+updating = false\n\s+applyResult\(data\)/)
    assert.match(widgetSource,
        /visible: root\.batteryPercent < 0\n\s+text: "—"/)
    assert.match(widgetSource,
        /: root\.batteryPercent >= 0 \? `\$\{root\.batteryPercent\}%` : "—"/)
    // Off, the dongle's last reading is stale: no percent is shown.
    assert.match(widgetSource,
        /text: root\.headsetConnected === false\n\s+\? \(root\.charging \? "Desligado · carregando" : "Desligado"\)/)
})
