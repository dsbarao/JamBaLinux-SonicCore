import assert from "node:assert/strict"
import { access, readFile } from "node:fs/promises"
import test from "node:test"

const appDir = new URL("../packaging/app/", import.meta.url)
const appSource = await readFile(new URL("main.qml", appDir), "utf8")

const match = appSource.match(/readonly property url trayIcon: \{([\s\S]*?)\n    \}\n/)
assert.notEqual(match, null, "the app must pick its tray icon in trayIcon")
const pick = new Function("core", "Qt", match[1])
const Qt = { resolvedUrl: path => path }

test("every battery state maps to an icon file that exists", async () => {
    const states = [{ headsetConnected: false, batteryPercent: 50, charging: false },
        { headsetConnected: null, batteryPercent: -1, charging: false }]
    for (let percent = 0; percent <= 100; percent++)
        for (const charging of [false, true])
            states.push({ headsetConnected: true, batteryPercent: percent, charging })
    for (const core of states) {
        const path = pick(core, Qt)
        await access(new URL(path, appDir))
    }
})

test("the tray icon follows the widget's battery colours", async () => {
    const svg = async name => readFile(new URL(`icons/${name}.svg`, appDir), "utf8")
    assert.match(await svg("battery-65-charging"), /#22d3ee/)
    assert.match(await svg("battery-65"), /#35c759/)
    assert.match(await svg("battery-45"), /#f5c542/)
    assert.match(await svg("battery-15"), /#ff453a/)
    assert.equal(pick({ headsetConnected: false, batteryPercent: 80, charging: false }, Qt),
        "icons/battery-unknown.svg")
    assert.equal(pick({ headsetConnected: false, batteryPercent: 65, charging: true }, Qt),
        "icons/battery-unknown-charging.svg")
    assert.equal(pick({ headsetConnected: true, batteryPercent: 63, charging: true }, Qt),
        "icons/battery-65-charging.svg")
})
