import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const base = new URL("../packaging/app/contents/", import.meta.url)
const mainSource = await readFile(new URL("ui/SonicCore.qml", base), "utf8")
// The app opens the settings pages itself (there is no Plasma settings dialog).
const configModel = await readFile(new URL("../main.qml", base), "utf8")
    + await readFile(new URL("ui/SonicCore.qml", base), "utf8")
const batteryPage = await readFile(new URL("ui/configBattery.qml", base), "utf8")

const hoursMatch = mainSource.match(/readonly property string hoursLeftText: ([\s\S]*?)\n\s+: ""/)
assert.notEqual(hoursMatch, null, "SonicCore.qml must expose hoursLeftText")

function hoursLeftText(hoursLeft, charging) {
    return new Function("hoursLeft", "charging", `return ${hoursMatch[1]}\n: ""`)(hoursLeft, charging)
}

test("o tempo restante aparece só fora do carregador e com estimativa", () => {
    assert.equal(hoursLeftText(17.6, false), "~18 h restantes")
    assert.equal(hoursLeftText(0.4, false), "~24 min restantes")
    assert.equal(hoursLeftText(17.6, true), "")
    assert.equal(hoursLeftText(-1, false), "")
})

test("carregando, aparece quanto falta para encher", () => {
    const toFull = mainSource.match(/readonly property string hoursToFullText: ([\s\S]*?)\n\s+: ""/)
    const duration = mainSource.match(/function durationText\(hours\) \{([\s\S]*?)\n    \}/)
    assert.notEqual(toFull, null)
    assert.notEqual(duration, null)
    const durationText = new Function("hours", duration[1])
    const text = (hoursToFull, charging, batteryPercent) => new Function(
        "hoursToFull", "charging", "batteryPercent", "durationText",
        `return ${toFull[1]}\n: ""`)(hoursToFull, charging, batteryPercent, durationText)
    assert.equal(text(4.47, true, 45), "~4 h 30 min para carregar")
    assert.equal(text(2, true, 70), "~2 h para carregar")
    assert.equal(text(0.6, true, 90), "~40 min para carregar")
    assert.equal(text(0.05, true, 95), "~10 min para carregar")
    assert.equal(text(4.47, false, 45), "", "only while charging")
    assert.equal(text(4.47, true, 100), "", "full")
    assert.equal(text(-1, true, 45), "", "no measured speed yet")
    // The same line shows one or the other.
    assert.match(mainSource, /readonly property string batteryTimeText: charging \? hoursToFullText : hoursLeftText/)
    assert.match(mainSource, /hoursLeftText: root\.batteryTimeText/)
    assert.match(mainSource, /root\.hoursToFull = Number\.isFinite\(toFull\) \? toFull : -1/)
})

test("o widget avisa quando a economia apagou as luzes", () => {
    assert.match(mainSource, /powerSaving = String\(result\.power_saving \?\? ""\)/)
    assert.match(mainSource, /visible: root\.powerSaving\.length > 0/)
    assert.match(mainSource, /soniccoreCommand\(\["power", "--format", "json"\]\)/)
})

test("a página Bateria aplica pela CLI só valores fixos e nunca o ANC", () => {
    assert.match(configModel, /batterySettingsPage: Qt\.resolvedUrl\("configBattery\.qml"\)/)
    assert.match(configModel, /window\.openSettings\(core\.batterySettingsPage, "Bateria"\)/)
    assert.match(batteryPage, /readonly property var lowBatteryChoices: \["off", "10", "15", "20", "30"\]/)
    assert.match(batteryPage, /readonly property var idleChoices: \["off", "5", "10", "15", "30"\]/)
    assert.match(batteryPage, /run\(\["power", "set", setting, value\]\)/)
    assert.doesNotMatch(batteryPage, /ambient/)
})

test("a página Saída de som escolhe pela CLI, com aspas, e explica o que fica no fone", async () => {
    const outputPage = await readFile(new URL("ui/configOutput.qml", base), "utf8")
    assert.match(configModel, /outputSettingsPage: Qt\.resolvedUrl\("configOutput\.qml"\)/)
    assert.match(configModel, /window\.openSettings\(core\.outputSettingsPage, "Saída de áudio"\)/)
    assert.match(outputPage, /run\(\["output", "set", target\]\)/)
    assert.match(outputPage, /arguments\.map\(shellQuote\)/)
    assert.match(outputPage, /target: output\.quantum_game \? "quantum-game" : output\.name/)
    assert.match(outputPage, /canal de Chat do fone nunca é usado/)
    assert.match(outputPage, /target: "auto",\n\s+label: "Automático \(agora: "/)
})
