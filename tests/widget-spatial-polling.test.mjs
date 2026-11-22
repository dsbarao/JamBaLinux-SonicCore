import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/SonicCore.qml",
    import.meta.url
)
const widgetSource = await readFile(widgetPath, "utf8")

test("spatial polling timer runs only when popup is expanded and spatial section is open", () => {
    // Find the Timer block that calls root.refreshSpatial()
    const spatialTimerMatch = widgetSource.match(
        /Timer\s*\{[^}]*?repeat:\s*true[^}]*?onTriggered:\s*root\.refreshSpatial\(\)[^}]*\}/
    )
    assert.ok(
        spatialTimerMatch,
        "spatial polling Timer calling root.refreshSpatial() should exist in SonicCore.qml"
    )

    const timerBlock = spatialTimerMatch[0]

    // Every section is on screen with the panel: poll only while it is shown
    assert.match(
        timerBlock,
        /running:\s*root\.expanded\n/,
        "spatial Timer running property must be 'root.expanded'"
    )

    // Verify triggeredOnStart is true
    assert.match(
        timerBlock,
        /triggeredOnStart:\s*true/,
        "spatial Timer must have triggeredOnStart: true"
    )

    // Verify repeat is true
    assert.match(
        timerBlock,
        /repeat:\s*true/,
        "spatial Timer must have repeat: true"
    )

    // Verify interval definition is preserved
    assert.match(
        timerBlock,
        /interval:\s*\(root\.spatialEnabled\s*&&\s*!root\.spatialActive\)\s*\|\|\s*root\.spatialMixPending\s*\?\s*1000\s*:\s*15000/,
        "spatial Timer interval must be preserved"
    )
})

test("a leitura periódica do espacial não mexe na grade dos modos (sem piscar)", async () => {
    const { readFile } = await import("node:fs/promises")
    const source = await readFile(new URL(
        "../packaging/plasma/org.jambalinux.soniccore/contents/ui/SonicCore.qml", import.meta.url), "utf8")
    assert.match(source, /PlasmaComponents\.BusyIndicator \{\n\s+running: root\.spatialBusy\n\s+visible: running/)
    assert.doesNotMatch(source, /running: root\.spatialBusy \|\| root\.spatialUpdating/)
})
