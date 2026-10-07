import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/main.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

const applyResultMatch = widgetSource.match(
    /    function applyResult\(data\) \{([\s\S]*?)^    \}\n\n    compactRepresentation/m)
assert.notEqual(applyResultMatch, null, "main.qml must apply status results")
const applyResult = new Function("normalizeBatteryPercent", "friendlyError",
    `return function applyResult(data) { with (this) {${applyResultMatch[1]}} };`)(
    () => -1, () => "")

function statusWithGameChat(value) {
    return {
        "exit code": 0,
        stdout: JSON.stringify({
            game_chat_value: value
        })
    }
}

function createWidgetDouble() {
    return {
        batteryPercent: -1, charging: false, rawFeature: "", ambientMode: "",
        headsetConnected: null, microphoneState: "", sidetoneLevel: "",
        lightingEnabled: null, lightingColor: "", logoColor: "", ringColor: "",
        logoColors: [], ringColors: [], logoEffect: "", ringEffect: "",
        logoSpeed: "", ringSpeed: "", gameChatValue: -999, errorMessage: ""
    }
}

test("valid integer game_chat_value from 0 to 16 is recorded as number", () => {
    for (const [input, expected] of [[0, 0], [8, 8], [16, 16], ["12", 12]]) {
        const widget = createWidgetDouble()
        applyResult.call(widget, statusWithGameChat(input))
        assert.strictEqual(widget.gameChatValue, expected)
    }
})

test("absent or invalid game_chat_value results in -1", () => {
    const invalidValues = [null, undefined, "", "abc", 3.5, -1, 17, true]
    for (const value of invalidValues) {
        const widget = createWidgetDouble()
        if (value === undefined) {
            applyResult.call(widget, { "exit code": 0, stdout: JSON.stringify({}) })
        } else {
            applyResult.call(widget, statusWithGameChat(value))
        }
        assert.strictEqual(widget.gameChatValue, -1, `Expected -1 for ${JSON.stringify(value)}`)
    }
})
