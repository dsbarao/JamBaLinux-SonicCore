import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/app/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

function extract(name) {
    const match = widgetSource.match(
        new RegExp(`    function ${name}\\(([^)]*)\\) \\{([\\s\\S]*?)^    \\}\\n`, "m"))
    assert.notEqual(match, null, `main.qml must expose ${name}`)
    return new Function(...match[1].split(",").map(arg => arg.trim()).filter(Boolean),
        `with (this) {${match[2]}}`)
}

const refresh = extract("refresh")
const finishRefresh = extract("finishRefresh")

function createWidgetDouble() {
    const widget = {
        updating: false,
        refreshPending: false,
        errorMessage: "",
        cachedCommand: "soniccore cached-status --format json",
        reads: 0,
        applied: [],
        timerRestarts: 0,
        cachedState: { connectSource() { widget.reads += 1 } },
        refreshAfterControl: { restart() { widget.timerRestarts += 1 } },
        applyResult(data) { widget.applied.push(data) }
    }
    return widget
}

test("um refresh pedido durante outro é reagendado, não descartado", () => {
    const widget = createWidgetDouble()
    refresh.call(widget)
    assert.equal(widget.reads, 1)

    // A second daemon signal arrives while the first read is running.
    refresh.call(widget)
    assert.equal(widget.reads, 1, "não deve iniciar duas leituras ao mesmo tempo")
    assert.equal(widget.refreshPending, true)

    finishRefresh.call(widget, { "exit code": 0, stdout: "{}" })
    assert.equal(widget.updating, false)
    assert.equal(widget.applied.length, 1)
    assert.equal(widget.refreshPending, false)
    assert.equal(widget.timerRestarts, 1, "a leitura pendente deve ser agendada")
})

test("sem pedido pendente, terminar a leitura não agenda outra", () => {
    const widget = createWidgetDouble()
    refresh.call(widget)
    finishRefresh.call(widget, { "exit code": 0, stdout: "{}" })
    assert.equal(widget.timerRestarts, 0)
})

test("controles ficam indisponíveis com o headset desligado", () => {
    const match = widgetSource.match(/readonly property bool deviceAvailable: (.*)$/m)
    assert.notEqual(match, null)
    const available = (batteryPercent, headsetConnected) =>
        new Function("batteryPercent", "headsetConnected", `return ${match[1]}`)(
            batteryPercent, headsetConnected)
    assert.equal(available(75, false), false)
    assert.equal(available(75, null), true)
    assert.equal(available(75, true), true)
    assert.equal(available(-1, true), false)
})
