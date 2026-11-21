import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

const clearHeadsetStateMatch = widgetSource.match(
    /    function clearHeadsetState\(\) \{([\s\S]*?)^    \}\n\n    function applyResult/m)
const applyResultMatch = widgetSource.match(
    /    function applyResult\(data\) \{([\s\S]*?)^    \}\n\n    property Component compactRepresentation/m)

assert.notEqual(clearHeadsetStateMatch, null, "SonicCore.qml must declare clearHeadsetState before applyResult")
assert.notEqual(applyResultMatch, null, "SonicCore.qml must apply results")

const clearHeadsetStateBody = clearHeadsetStateMatch[1]
const applyResultBody = applyResultMatch[1]

const friendlyError = (msg, exitCode, context) => msg === "error_stderr" ? "Sem permissão para acessar o headset" : "Não foi possível ler o headset"

function createApplyResult() {
    return new Function("friendlyError", "normalizeBatteryPercent", `
        return function applyResult(data) {
            const self = this;
            function clearHeadsetState() {
                with (self) {
                    ${clearHeadsetStateBody}
                }
            }
            with (this) {
                ${applyResultBody}
            }
        };
    `)(friendlyError, (v) => v)
}

const applyResult = createApplyResult()

function createPopulatedWidget() {
    return {
        batteryPercent: 85,
        charging: true,
        headsetConnected: true,
        rawFeature: "raw_data_present",
        ambientMode: "anc",
        microphoneState: "muted",
        sidetoneLevel: "high",
        lightingEnabled: true,
        lightingColor: "#ff00cc",
        logoColor: "#ff00cc",
        ringColor: "#0029ff",
        logoColors: ["#ff00cc", "#ff00cc", "#ff00cc", "#ff00cc", "#ff00cc"],
        ringColors: ["#0029ff", "#0029ff", "#0029ff", "#0029ff", "#0029ff"],
        logoEffect: "breathing",
        ringEffect: "spectrum",
        logoSpeed: "1.0",
        ringSpeed: "2.0",
        gameChatValue: 50,
        errorMessage: ""
    }
}

function assertHeadsetStateIsUnknown(widget) {
    assert.equal(widget.batteryPercent, -1)
    assert.equal(widget.charging, false)
    assert.equal(widget.headsetConnected, null)
    assert.equal(widget.rawFeature, "")
    assert.equal(widget.ambientMode, "unknown")
    assert.equal(widget.microphoneState, "unknown")
    assert.equal(widget.sidetoneLevel, "unknown")
    assert.equal(widget.lightingEnabled, null)
    assert.equal(widget.lightingColor, null)
    assert.equal(widget.logoColor, null)
    assert.equal(widget.ringColor, null)
    assert.equal(widget.logoColors, null)
    assert.equal(widget.ringColors, null)
    assert.equal(widget.logoEffect, null)
    assert.equal(widget.ringEffect, null)
    assert.equal(widget.logoSpeed, null)
    assert.equal(widget.ringSpeed, null)
    assert.equal(widget.gameChatValue, -1)
}

test("non-zero exit code sets all headset fields to unknown and preserves friendlyError", () => {
    const widget = createPopulatedWidget()
    applyResult.call(widget, {
        "exit code": 1,
        stdout: "",
        stderr: "error_stderr"
    })

    assertHeadsetStateIsUnknown(widget)
    assert.equal(widget.errorMessage, "Sem permissão para acessar o headset")
})

test("invalid JSON sets all headset fields to unknown and records friendly error message", () => {
    const widget = createPopulatedWidget()
    applyResult.call(widget, {
        "exit code": 0,
        stdout: "invalid json string {not valid}",
        stderr: ""
    })

    assertHeadsetStateIsUnknown(widget)
    assert.match(widget.errorMessage, /^Resposta inválida:/)
})
