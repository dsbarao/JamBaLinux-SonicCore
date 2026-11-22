import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL("../packaging/app/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

function extract(name) {
    const match = widgetSource.match(
        new RegExp(`    function ${name}\\(([^)]*)\\) \\{([\\s\\S]*?)^    \\}\\n`, "m"))
    assert.notEqual(match, null, `SonicCore.qml must expose ${name}`)
    return new Function(...match[1].split(",").map(arg => arg.trim()).filter(Boolean),
        `with (this) {${match[2]}}`)
}

const names = ["radioTune", "radioSkip", "radioNext", "radioPrevious", "toggleRadioFavorite",
    "radioStationLine", "radioStart"]
const functions = Object.fromEntries(names.map(name => [name, extract(name)]))

const stations = [
    { uuid: "a", name: "Synthradio", url: "http://a", country: "RU", bitrate: 192 },
    { uuid: "b", name: "Outrun FM", url: "http://b", country: "", bitrate: 0 },
    { uuid: "c", name: "Retro", url: "http://c", country: "BR", bitrate: 128 },
]

function radioDouble() {
    const radio = {
        radioOn: true,
        radioStations: stations,
        radioIndex: -1,
        radioFailures: 0,
        radioMessage: "",
        radioFavorites: [],
        configuration: { radioLastStation: "", radioFavorites: "[]" },
        played: [],
        paused: [],
        stopped: 0,
        radioTitle: { clear() {} },
        Sonic: { RadioPlayer: { service: "org.mpris.MediaPlayer2.jambalinux_soniccore" } },
        media: { pauseOthers(except) { radio.paused.push(except) } },
        radioPlayer: {
            set source(url) { radio.played.push(url) },
            play() {},
            stop() { radio.stopped += 1 },
        },
        radioWatchdog: { restart() {}, stop() {} },
        get radioStation() { return this.radioStations[this.radioIndex] ?? null },
        get radioFavorite() {
            return this.radioStation !== null && this.radioFavorites.some(s => s.uuid === this.radioStation.uuid)
        },
    }
    for (const [name, body] of Object.entries(functions))
        radio[name] = (...args) => body.call(radio, ...args)
    return radio
}

test("trocar de estação dá a volta na lista e lembra a última", () => {
    const radio = radioDouble()
    radio.configuration.radioLastStation = "c"
    radio.radioStart()
    assert.equal(radio.radioIndex, 2)
    radio.radioNext()
    assert.equal(radio.radioIndex, 0)
    radio.radioPrevious()
    assert.equal(radio.radioIndex, 2)
    assert.deepEqual(radio.played, ["http://c", "http://a", "http://c"])
    assert.equal(radio.configuration.radioLastStation, "c")
    // Other players pause when a station starts.
    assert.equal(radio.paused[0], "org.mpris.MediaPlayer2.jambalinux_soniccore")
})

test("estações que não respondem são puladas, e todas falhando param com aviso", () => {
    const radio = radioDouble()
    radio.radioTune(0)
    radio.radioSkip()
    assert.equal(radio.radioIndex, 1)
    radio.radioSkip()
    assert.equal(radio.radioIndex, 2)
    radio.radioSkip()   // the third failure of three stations
    assert.equal(radio.radioIndex, 2)
    assert.equal(radio.stopped, 1)
    assert.match(radio.radioMessage, /não responderam/)
    // Choosing a station by hand starts counting again.
    radio.radioNext()
    assert.equal(radio.radioFailures, 0)
})

test("favoritar guarda a estação e desfavoritar tira", () => {
    const radio = radioDouble()
    radio.radioTune(2)
    radio.toggleRadioFavorite()
    const saved = JSON.parse(radio.configuration.radioFavorites)
    assert.deepEqual(saved.map(station => station.uuid), ["c"])
    assert.deepEqual(Object.keys(saved[0]).sort(), ["bitrate", "country", "favicon", "name", "url", "uuid"])
    radio.radioFavorites = saved
    radio.toggleRadioFavorite()
    assert.deepEqual(JSON.parse(radio.configuration.radioFavorites), [])
})

test("a linha da estação mostra país e qualidade quando existem", () => {
    const radio = radioDouble()
    assert.equal(radio.radioStationLine(stations[0]), "Synthradio · RU · 192 kbps")
    assert.equal(radio.radioStationLine(stations[1]), "Outrun FM")
    assert.equal(radio.radioStationLine(null), "")
})

test("cada estilo tem etiquetas do catálogo e Favoritas usa as guardadas", () => {
    const styles = widgetSource.match(/readonly property var radioStyles: \[([\s\S]*?)\n    \]/)[1]
    for (const label of ["Rock", "Heavy Metal", "Synthwave", "Anos 80", "Lo-fi", "MPB", "Favoritas"])
        assert.match(styles, new RegExp(`"label": "${label}"`))
    assert.match(widgetSource, /radioStations = radioFavorites/)
    // The radio is a player of the desktop while it is on.
    assert.match(widgetSource, /Sonic\.RadioPlayer\.active = on/)
    assert.match(widgetSource, /function onNextRequested\(\) \{ root\.radioNext\(\) \}/)
})
