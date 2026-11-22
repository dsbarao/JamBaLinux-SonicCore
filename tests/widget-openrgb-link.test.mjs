import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const source = await readFile(new URL(
    "../packaging/app/contents/ui/SonicCore.qml", import.meta.url), "utf8")

function extract(name) {
    const match = source.match(new RegExp(`    function ${name}\\(([^)]*)\\) \\{([\\s\\S]*?)^    \\}\\n`, "m"))
    assert.notEqual(match, null, `SonicCore.qml must expose ${name}`)
    return new Function(...match[1].split(",").map(arg => arg.trim()).filter(Boolean),
        `with (this) {${match[2]}}`)
}

const canLink = extract("openRgbCanLink")
const summary = extract("openRgbSummary")
const ready = {
    schema: 1, openrgb_installed: true, openrgb_version: "1.1", supported_versions: ["1.1"],
    supported: true, openrgb_running: false, plugin_installed: false, plugin_current: null,
    plugin_enabled: null, missing_tools: []
}
const state = status => ({ openRgbStatus: status, openRgbLinking: false, openRgbMessage: "" })

test("o botão Vincular só aparece quando dá para instalar ou atualizar", () => {
    assert.equal(canLink.call(state(ready)), true)
    assert.equal(canLink.call(state({ ...ready, plugin_installed: true, plugin_current: false })), true)
    assert.equal(canLink.call(state({ ...ready, plugin_installed: true, plugin_current: true })), false)
    assert.equal(canLink.call(state({ ...ready, plugin_installed: true, plugin_current: null })), false)
    assert.equal(canLink.call(state({ ...ready, supported: false })), false)
    assert.equal(canLink.call(state({ ...ready, missing_tools: ["qmake6"] })), false)
    assert.equal(canLink.call(state({ ...ready, openrgb_installed: false })), false)
    assert.equal(canLink.call(state(null)), false)
})

test("o texto explica o que falta em linguagem simples", () => {
    assert.match(summary.call(state({ ...ready, openrgb_installed: false })), /Instale o OpenRGB/)
    assert.match(summary.call(state({ ...ready, supported: false, openrgb_version: "0.9" })), /0\.9 ainda não é suportado/)
    assert.match(summary.call(state({ ...ready, missing_tools: ["qmake6", "g++"] })), /instale antes: qmake6, g\+\+/)
    assert.match(summary.call(state({ ...ready, plugin_installed: true, plugin_current: true })), /^Vinculado ao OpenRGB/)
    assert.match(summary.call(state({ ...ready, plugin_installed: true, plugin_enabled: false })), /desativado/)
    assert.match(summary.call({ ...state(ready), openRgbLinking: true }), /compilando/)
})

test("o resultado do vínculo é reconhecido pelo comando exato", () => {
    assert.match(source, /readonly property string openRgbLinkCommand: soniccoreCommand\(\["openrgb", "link", "--format", "json"\]\)/)
    assert.match(source, /if \(sourceName === openRgbLinkCommand\)/)
})
