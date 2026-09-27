import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/main.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

function extractFunction(name, nextName) {
    const match = widgetSource.match(new RegExp(
        `    function ${name}\\(([^)]*)\\) \\{([\\s\\S]*?)^    \\}\\n\\n    function ${nextName}`, "m"))
    assert.notEqual(match, null, `main.qml must expose ${name}`)
    return `function ${name}(${match[1]}) {${match[2]}}`
}

const applyResultMatch = widgetSource.match(
    /    function applyResult\(data\) \{([\s\S]*?)^    \}\n\n    compactRepresentation/m)
assert.notEqual(applyResultMatch, null, "main.qml must apply status results")
const applyResult = new Function("normalizeBatteryPercent", "friendlyError",
    `return function applyResult(data) { with (this) {${applyResultMatch[1]}} };`)(
    () => -1, () => "")

const lightingHelpers = new Function(`with (this) {
${extractFunction("lightingFeature", "lightingEffectFeature")}
${extractFunction("zoneProfileKnown", "lightingTargetProfileKnown")}
${extractFunction("lightingTargetProfileKnown", "canApplyLightingColor")}
${extractFunction("canApplyLightingColor", "normalizeColor")}
return { lightingFeature, zoneProfileKnown, lightingTargetProfileKnown, canApplyLightingColor }
}`)

function unknownLightingStatus() {
    return {
        "exit code": 0,
        stdout: JSON.stringify({
            battery_percent: 50,
            logo_colors: null, logo_effect: null, logo_speed: null,
            ring_colors: ["#112233", "#112233", "#112233", "#112233", "#112233"],
            ring_effect: "wave", ring_speed: "1.5"
        })
    }
}

test("unknown lighting status remains null without profile fallbacks", () => {
    const widget = {
        batteryPercent: -1, charging: false, rawFeature: "", ambientMode: "",
        headsetConnected: null, microphoneState: "", sidetoneLevel: "",
        lightingEnabled: null, lightingColor: "old", logoColor: "old", ringColor: "old",
        logoColors: ["#33ffcc"], ringColors: ["#33ffcc"], logoEffect: "solid",
        ringEffect: "solid", logoSpeed: "0.5", ringSpeed: "0.5",
        gameChatValue: -1, errorMessage: ""
    }
    applyResult.call(widget, unknownLightingStatus())

    assert.equal(widget.logoColors, null)
    assert.equal(widget.logoEffect, null)
    assert.equal(widget.logoSpeed, null)
    assert.notEqual(widget.logoColors, "#33ffcc")
    assert.notEqual(widget.logoEffect, "solid")
    assert.notEqual(widget.logoSpeed, "0.5")
})

test("unknown zone disables cached commands but preserves synchronized recovery", () => {
    const widget = {
        logoColors: null, logoEffect: null, logoSpeed: null,
        ringColors: ["#112233", "#112233", "#112233", "#112233", "#112233"],
        ringEffect: "wave", ringSpeed: "1.5", lightingTarget: "both", lightingSegment: 2
    }
    const helpers = lightingHelpers.call(widget)

    assert.equal(helpers.zoneProfileKnown("logo"), false)
    assert.equal(helpers.zoneProfileKnown("ring"), true)
    assert.equal(helpers.lightingTargetProfileKnown(), false)
    assert.equal(helpers.canApplyLightingColor(), true)
    assert.equal(helpers.lightingFeature(), "color")

    widget.lightingTarget = "logo"
    assert.equal(helpers.canApplyLightingColor(), false)
    widget.lightingTarget = "ring"
    assert.equal(helpers.canApplyLightingColor(), false)
})

test("segment swatches guard null profiles and the effect row follows its heading", () => {
    assert.match(widgetSource,
        /color: Array\.isArray\(colors\) \? colors\[index\] \?\? "transparent" : "transparent"/)
    assert.match(widgetSource,
        /visible: root\.openSection === "lighting" && root\.lightingTargetProfileKnown\(\)\n\s+enabled: root\.deviceAvailable && !root\.controlBusy && root\.lightingTargetProfileKnown\(\)/)
})
