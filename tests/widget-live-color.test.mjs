import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

function extract(name) {
    const match = widgetSource.match(
        new RegExp(`    function ${name}\\(([^)]*)\\) \\{([\\s\\S]*?)^    \\}\\n`, "m"))
    assert.notEqual(match, null, `main.qml must expose ${name}`)
    return new Function(...match[1].split(",").map(arg => arg.trim()).filter(Boolean),
        `with (this) {${match[2]}}`)
}

const queueLightingColor = extract("queueLightingColor")
const flushLightingColor = extract("flushLightingColor")

function createWidgetDouble() {
    const widget = {
        lightingWritable: true,
        controlBusy: false,
        pendingLightingFeature: "",
        pendingLightingColor: "",
        lastLiveLightingKey: "",
        color: "#ff0000",
        sent: [],
        throttleRunning: false,
        canApplyLightingColor: () => true,
        lightingFeature: () => "logo-color",
        pickerHex: () => widget.color,
        soniccoreCommand: args => args.join(" "),
        executable: { connectSource(command) { widget.sent.push(command) } },
        lightingColorThrottle: {
            get running() { return widget.throttleRunning },
            restart() { widget.throttleRunning = true }
        }
    }
    widget.queueLightingColor = () => queueLightingColor.call(widget)
    widget.flushLightingColor = () => flushLightingColor.call(widget)
    return widget
}

test("a primeira cor é enviada na hora, sem botão Aplicar", () => {
    const widget = createWidgetDouble()
    widget.queueLightingColor()
    assert.deepEqual(widget.sent, ["set logo-color #ff0000"])
    assert.equal(widget.controlBusy, true)
    assert.doesNotMatch(widgetSource, /text: "Aplicar"/)
})

test("durante o arraste só a cor mais recente espera e é enviada depois", () => {
    const widget = createWidgetDouble()
    widget.queueLightingColor()
    for (const color of ["#ee0000", "#dd0000", "#cc0000"]) {
        widget.color = color
        widget.queueLightingColor()
    }
    assert.equal(widget.sent.length, 1, "nada é enviado enquanto o comando roda")
    assert.equal(widget.pendingLightingColor, "#cc0000")

    // The running command finishes and the throttle expires.
    widget.controlBusy = false
    widget.throttleRunning = false
    widget.flushLightingColor()
    assert.deepEqual(widget.sent, ["set logo-color #ff0000", "set logo-color #cc0000"])
    assert.equal(widget.pendingLightingColor, "")
})

test("a mesma cor não é enviada duas vezes seguidas", () => {
    const widget = createWidgetDouble()
    widget.queueLightingColor()
    widget.controlBusy = false
    widget.throttleRunning = false
    widget.queueLightingColor()
    assert.equal(widget.sent.length, 1)
})

test("sem headset disponível, ou com o OpenRGB no controle, nada é enviado", () => {
    const widget = createWidgetDouble()
    widget.lightingWritable = false
    widget.queueLightingColor()
    assert.equal(widget.sent.length, 0)
})

test("o seletor continua ativo enquanto um comando roda", () => {
    assert.match(widgetSource,
        /id: colorPicker[\s\S]{0,600}enabled: root\.lightingWritable && root\.canApplyLightingColor\(\)/)
})

test("escolher um matiz partindo do preto gera uma cor visível", () => {
    const match = widgetSource.match(
        /root\.pickerHue = \(Math\.atan2\(offsetY, offsetX\)[^\n]*\n([\s\S]*?)root\.queueLightingColor\(\)/)
    assert.notEqual(match, null)
    const root = { pickerValue: 0, pickerSaturation: 0 }
    const canvas = { requestPaint() {} }
    new Function("root", "svCanvas", match[1])(root, canvas)
    assert.equal(root.pickerValue, 1)
    assert.equal(root.pickerSaturation, 1)

    // A color the user already set keeps its own brightness and saturation.
    const set = { pickerValue: 0.4, pickerSaturation: 0.6 }
    new Function("root", "svCanvas", match[1])(set, canvas)
    assert.deepEqual(set, { pickerValue: 0.4, pickerSaturation: 0.6 })
})
