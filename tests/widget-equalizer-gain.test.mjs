import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/app/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

function extract(name) {
    const match = widgetSource.match(
        new RegExp(`    function ${name}\\(([^)]*)\\) \\{([\\s\\S]*?)^    \\}\\n`, "m"))
    assert.notEqual(match, null, `SonicCore.qml must expose ${name}`)
    return new Function(...match[1].split(",").map(arg => arg.trim()).filter(Boolean),
        `with (this) {${match[2]}}`)
}

const functions = Object.fromEntries(["setEqualizerGain", "sendNextEqualizerBand",
    "showEqualizerGain", "finishEqualizerSet"].map(name => [name, extract(name)]))

function createWidgetDouble() {
    const widget = {
        equalizerBusy: false,
        pendingEqualizerAction: "",
        equalizerActionError: "",
        pendingEqualizerBands: {},
        pendingEqualizerGain: null,
        equalizerSetCommand: "",
        equalizerPreampDb: 0,
        equalizerPreampAuto: false,
        equalizerDspPreampDb: 0,
        equalizerVisualRevision: 0,
        equalizerActiveProfileId: "",
        sent: [],
        refreshes: 0,
        equalizerFullRefresh: { stop() {}, restart() {} },
        equalizerExecutable: { connectSource(command) { widget.sent.push(command) } },
        equalizerCliCommand: args => args.join(" "),
        showConfirmedEqualizerBands() {},
        equalizerActionFailureMessage: () => "falhou",
        refreshEqualizer() { widget.refreshes += 1 },
    }
    for (const [name, body] of Object.entries(functions))
        widget[name] = (...args) => body.call(widget, ...args)
    return widget
}

test("o ganho arrastado envia só o valor mais recente, como as bandas", () => {
    const widget = createWidgetDouble()
    widget.setEqualizerGain(-3.04)
    assert.deepEqual(widget.sent, ["gain --format json -3"])

    // Two more moves while the first runs: only the newest is kept.
    widget.setEqualizerGain(-5)
    widget.setEqualizerGain(-6.5)
    assert.equal(widget.sent.length, 1)
    assert.equal(widget.pendingEqualizerGain, -6.5)

    widget.finishEqualizerSet(0, JSON.stringify({ applied_bands: [], preamp_db: -3,
        preamp_auto: false, dsp_preamp_db: -3 }), "")
    assert.deepEqual(widget.sent.at(-1), "gain --format json -6.5")
    // The older answer does not move the handle back while a newer one waits.
    assert.equal(widget.equalizerDspPreampDb, 0)

    widget.finishEqualizerSet(0, JSON.stringify({ applied_bands: [], preamp_db: -6.5,
        preamp_auto: false, dsp_preamp_db: -6.5 }), "")
    assert.equal(widget.equalizerDspPreampDb, -6.5)
    assert.equal(widget.equalizerBusy, false)
})

test("Auto mostra o ganho que ele escolheu", () => {
    const widget = createWidgetDouble()
    widget.setEqualizerGain("auto")
    assert.deepEqual(widget.sent, ["gain --format json auto"])
    widget.finishEqualizerSet(0, JSON.stringify({ applied_bands: [], preamp_db: 0,
        preamp_auto: true, dsp_preamp_db: -12 }), "")
    assert.equal(widget.equalizerPreampAuto, true)
    assert.equal(widget.equalizerDspPreampDb, -12)
})
