import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

const normalizerMatch = widgetSource.match(
    /    function normalizeBatteryPercent\(value\) \{([\s\S]*?)^    \}\n\n    function/m)
const applyResultMatch = widgetSource.match(
    /    function applyResult\(data\) \{([\s\S]*?)^    \}\n\n    property Component compactRepresentation/m)
const batteryColorMatch = widgetSource.match(
    /readonly property color batteryColor:\s*([\s\S]*?)(?=\n\s+ListModel)/m)

assert.notEqual(normalizerMatch, null, "SonicCore.qml must expose normalizeBatteryPercent")
assert.notEqual(applyResultMatch, null, "SonicCore.qml must expose applyResult")
assert.notEqual(batteryColorMatch, null, "SonicCore.qml must expose batteryColor")

const normalizeBatteryPercent = new Function(
    `function normalizeBatteryPercent(value) {${normalizerMatch[1]}}\nreturn normalizeBatteryPercent;`)()

const applyResult = new Function("normalizeBatteryPercent", "friendlyError", "normalizeColor",
    `return function applyResult(data) { with (this) {${applyResultMatch[1]}} };`)(
    normalizeBatteryPercent, () => "", () => "#33ffcc")

const Kirigami = {
    Theme: {
        disabledTextColor: "#888888"
    }
}

function evaluateBatteryColor({ headsetConnected, charging, batteryPercent }) {
    const fn = new Function("Kirigami", "headsetConnected", "charging", "batteryPercent",
        `return (${batteryColorMatch[1].trim()});`)
    return fn(Kirigami, headsetConnected, charging, batteryPercent)
}

function statusData(battery_percent, headset_connected, charging = false) {
    return {
        "exit code": 0,
        stdout: JSON.stringify({
            battery_percent,
            headset_connected,
            charging,
            ambient_mode: "anc",
            microphone: "muted",
            sidetone_level: "medium",
            lighting_enabled: true
        })
    }
}

function createWidgetDouble() {
    return {
        batteryPercent: 0,
        charging: false,
        rawFeature: "",
        ambientMode: "",
        headsetConnected: null,
        microphoneState: "",
        sidetoneLevel: "",
        lightingEnabled: null,
        lightingColor: "",
        logoColor: "",
        ringColor: "",
        logoColors: [],
        ringColors: [],
        logoEffect: "",
        ringEffect: "",
        logoSpeed: "",
        ringSpeed: "",
        gameChatValue: -1,
        errorMessage: ""
    }
}

test("battery_percent 0 com headset_connected null, ausente ou false vira -1", () => {
    for (const connected of [null, undefined, false]) {
        const widget = createWidgetDouble()
        applyResult.call(widget, statusData(0, connected))
        assert.equal(widget.batteryPercent, -1)
    }
})

test("battery_percent 0 com headset_connected true continua 0", () => {
    const widget = createWidgetDouble()
    applyResult.call(widget, statusData(0, true))
    assert.equal(widget.batteryPercent, 0)
})

test("battery_percent 75 com headset_connected null continua 75", () => {
    const widget = createWidgetDouble()
    applyResult.call(widget, statusData(75, null))
    assert.equal(widget.batteryPercent, 75)
})

test("battery_percent null continua -1 independentemente de headset_connected", () => {
    for (const connected of [null, undefined, false, true]) {
        const widget = createWidgetDouble()
        applyResult.call(widget, statusData(null, connected))
        assert.equal(widget.batteryPercent, -1)
    }
})

test("com batteryPercent -1 e charging true, batteryColor é cinza (disabledTextColor), não azul", () => {
    const color = evaluateBatteryColor({
        headsetConnected: null,
        charging: true,
        batteryPercent: -1
    })
    assert.equal(color, Kirigami.Theme.disabledTextColor)
})

test("com bateria conhecida e charging true, batteryColor é azul (#22d3ee)", () => {
    const color = evaluateBatteryColor({
        headsetConnected: true,
        charging: true,
        batteryPercent: 50
    })
    assert.equal(color, "#22d3ee")
})

test("sem carga e headset conectado, batteryColor segue os limites verde, amarelo e vermelho", () => {
    assert.equal(evaluateBatteryColor({ headsetConnected: true, charging: false, batteryPercent: 60 }), "#35c759")
    assert.equal(evaluateBatteryColor({ headsetConnected: true, charging: false, batteryPercent: 30 }), "#f5c542")
    assert.equal(evaluateBatteryColor({ headsetConnected: true, charging: false, batteryPercent: 29 }), "#ff453a")
    assert.equal(evaluateBatteryColor({ headsetConnected: true, charging: false, batteryPercent: 0 }), "#ff453a")
})

test("headsetConnected false é sempre cinza", () => {
    assert.equal(evaluateBatteryColor({ headsetConnected: false, charging: true, batteryPercent: 80 }), Kirigami.Theme.disabledTextColor)
    assert.equal(evaluateBatteryColor({ headsetConnected: false, charging: false, batteryPercent: 80 }), Kirigami.Theme.disabledTextColor)
    assert.equal(evaluateBatteryColor({ headsetConnected: false, charging: false, batteryPercent: -1 }), Kirigami.Theme.disabledTextColor)
})
