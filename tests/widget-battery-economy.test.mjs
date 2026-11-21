import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const base = new URL("../packaging/plasma/org.jambalinux.soniccore/contents/", import.meta.url)
const mainSource = await readFile(new URL("ui/main.qml", base), "utf8")
const configModel = await readFile(new URL("config/config.qml", base), "utf8")
const batteryPage = await readFile(new URL("ui/configBattery.qml", base), "utf8")

const hoursMatch = mainSource.match(/readonly property string hoursLeftText: ([\s\S]*?)\n\s+: ""/)
assert.notEqual(hoursMatch, null, "main.qml must expose hoursLeftText")

function hoursLeftText(hoursLeft, charging) {
    return new Function("hoursLeft", "charging", `return ${hoursMatch[1]}\n: ""`)(hoursLeft, charging)
}

test("o tempo restante aparece só fora do carregador e com estimativa", () => {
    assert.equal(hoursLeftText(17.6, false), "~18 h restantes")
    assert.equal(hoursLeftText(0.4, false), "~24 min restantes")
    assert.equal(hoursLeftText(17.6, true), "")
    assert.equal(hoursLeftText(-1, false), "")
})

test("o widget avisa quando a economia apagou as luzes", () => {
    assert.match(mainSource, /powerSaving = String\(result\.power_saving \?\? ""\)/)
    assert.match(mainSource, /visible: root\.openSection === "lighting" && root\.powerSaving\.length > 0/)
    assert.match(mainSource, /soniccoreCommand\(\["power", "--format", "json"\]\)/)
})

test("a página Bateria aplica pela CLI só valores fixos e nunca o ANC", () => {
    assert.match(configModel, /source: "configBattery\.qml"/)
    assert.match(batteryPage, /readonly property var lowBatteryChoices: \["off", "10", "15", "20", "30"\]/)
    assert.match(batteryPage, /readonly property var idleChoices: \["off", "5", "10", "15", "30"\]/)
    assert.match(batteryPage, /run\(\["power", "set", setting, value\]\)/)
    assert.doesNotMatch(batteryPage, /ambient/)
})

test("a página Saída de som escolhe pela CLI, com aspas, e explica o que fica no fone", async () => {
    const outputPage = await readFile(new URL("ui/configOutput.qml", base), "utf8")
    assert.match(configModel, /source: "configOutput\.qml"/)
    assert.match(outputPage, /run\(\["output", "set", target\]\)/)
    assert.match(outputPage, /arguments\.map\(shellQuote\)/)
    assert.match(outputPage, /target: output\.quantum_game \? "quantum-game" : output\.name/)
    assert.match(outputPage, /canal de Chat do fone nunca é usado/)
    assert.match(outputPage, /target: "auto",\n\s+label: "Automático — segue a saída padrão do sistema/)
})
