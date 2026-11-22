import { readFileSync } from "node:fs"
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const base = new URL("../packaging/plasma/org.jambalinux.soniccore/contents/", import.meta.url)
const mainSource = await readFile(new URL("ui/SonicCore.qml", base), "utf8")
const vivoSource = await readFile(new URL("ui/VivoPanel.qml", base), "utf8")
const configXml = await readFile(new URL("config/main.xml", base), "utf8")
const configPage = await readFile(new URL("ui/configAppearance.qml", base), "utf8")

function extract(source, name) {
    const match = source.match(new RegExp(`    function ${name}\\(([^)]*)\\) \\{([\\s\\S]*?)^    \\}\\n`, "m"))
    assert.notEqual(match, null, `${name} must exist`)
    return new Function(...match[1].split(",").map(arg => arg.trim()).filter(Boolean),
        `with (this) {${match[2]}}`)
}

test("o tema Leve é o padrão e a página de configuração oferece os dois", () => {
    assert.match(configXml, /<entry name="theme" type="String">\s*<default>leve<\/default>/)
    assert.match(configPage, /property string cfg_theme/)
    assert.match(configPage, /text: "Leve"/)
    assert.match(configPage, /text: "Vivo"/)
    assert.match(mainSource, /readonly property bool vivo: root\.configuration\.theme === "vivo"/)
})

test("o Vivo só substitui o topo e só roda com o painel aberto", () => {
    assert.match(mainSource, /id: vfdWindow\n\s+visible: root\.vivo/)
    assert.match(mainSource, /VivoPanel \{\n\s+id: vivoPanel\n/)
    assert.match(mainSource, /running: root\.vivo && root\.expanded/)
    // The Leve header (image and big battery label) hides only in Vivo.
    assert.match(mainSource, /Image \{\n\s+visible: !root\.vivo && root\.outputIsQuantum\n\s+source: root\.equipmentImage/)
    assert.match(mainSource, /PlasmaComponents\.Label \{\n\s+visible: !root\.vivo/)
    // Polling stops with the popup closed or the Leve theme.
    assert.match(vivoSource, /interval: 40\n\s+repeat: true\n\s+running: panel\.running/)
    assert.match(vivoSource, /running: panel\.running && panel\.pathCommand\.length > 0/)
    assert.match(vivoSource, /readonly property bool animate: Kirigami\.Units\.longDuration > 1/)
})

function panelDouble(path) {
    const panel = { path }
    panel.pathFlowing = extract(vivoSource, "pathFlowing").bind(panel)
    panel.linkState = extract(vivoSource, "linkState").bind(panel)
    panel.segmentStates = extract(vivoSource, "segmentStates").bind(panel)
    return panel
}

function path({ spatial = true, playing = true, links = {} } = {}) {
    return {
        stages: {
            streams: { count: 1, running: playing },
            spatial: { present: spatial, running: spatial },
            equalizer: { present: true, running: true },
            headset: { present: true, running: true },
        },
        links: Object.assign({
            streams_spatial: spatial, streams_equalizer: !spatial, streams_headset: false,
            spatial_equalizer: spatial, equalizer_headset: true,
        }, links),
    }
}

test("som passando pelo espacial acende os três trechos", () => {
    assert.deepEqual(panelDouble(path()).segmentStates(), ["flowing", "flowing", "flowing"])
})

test("sem espacial, os apps seguem direto para o EQ", () => {
    assert.deepEqual(panelDouble(path({ spatial: false })).segmentStates(), ["flowing", "flowing", "flowing"])
})

test("pausado fica parado e uma ligação quebrada fica vermelha", () => {
    assert.deepEqual(panelDouble(path({ playing: false })).segmentStates(), ["idle", "idle", "idle"])
    assert.deepEqual(panelDouble(path({ links: { spatial_equalizer: false } })).segmentStates(),
        ["flowing", "broken", "flowing"])
    assert.deepEqual(panelDouble(null).segmentStates(), ["off", "off", "off"])
})

test("cores pretas ou luzes apagadas usam a cor de destaque do sistema", () => {
    const vivoAccent = extract(mainSource, "vivoAccent")
    const normalizeColor = value => value
    const Kirigami = { Theme: { highlightColor: "accent" } }
    const call = (value, themeLightingEnabled = true) =>
        new Function("normalizeColor", "Kirigami", "themeLightingEnabled", "value",
            `return (${vivoAccent.toString()}).call({}, value)`)(normalizeColor, Kirigami, themeLightingEnabled, value)
    assert.equal(call("#59C319"), "#59c319")
    assert.equal(call("#000000"), "accent")
    assert.equal(call(null), "accent")
    assert.equal(call("#59C319", false), "accent")
})

test("com o fone desligado, os efeitos seguem as cores que o OpenRGB pede", () => {
    // The theme reads the headset's colors, or the OpenRGB plugin's file
    // while the headset is not connected.
    assert.match(mainSource, /readonly property bool usePcLighting: headsetConnected !== true && pcLighting !== null/)
    assert.match(mainSource, /pcColors: usePcLighting \? pcLighting\.colors\.map\(Palette\.fullBrightness\)/)
    assert.match(mainSource, /themeLogoColors: usePcLighting \? pcColors\.slice\(0, 5\)/)
    assert.match(mainSource, /themeRingColors: usePcLighting \? pcColors\.slice\(5, 10\)/)
    assert.match(mainSource, /vivoPalette: Palette\.build\(themeLightingEnabled, themeLogoColors,/)
    assert.match(mainSource, /"\/jambalinux-soniccore-openrgb\.json"/)
    const appScript = readFileSync(new URL("../packaging/app/soniccore-app", import.meta.url), "utf8")
    assert.match(appScript, /export QML_XHR_ALLOW_FILE_READ=1/)
})

test("o fundo do Vivo fica atrás de tudo, só no Vivo, e tem padrões seguros", () => {
    assert.match(configXml, /<entry name="background" type="String">\s*<default>aurora<\/default>/)
    assert.match(configXml, /<entry name="backgroundIntensity" type="Int">\s*<default>1<\/default>/)
    assert.match(configXml, /<entry name="backgroundReactive" type="Bool">\s*<default>true<\/default>/)
    // The background is the first child of the popup, behind the controls.
    assert.match(mainSource, /fullRepresentation: Item \{[\s\S]{0,1600}VivoBackground \{[\s\S]{0,200}visible: root\.vivo && !root\.hifi && root\.vivoBackground !== "none"/)
    assert.match(mainSource, /running: root\.vivo && !root\.hifi && root\.expanded\n\s+bass: vivoPanel\.bass/)
    // The wallpaper is only looked up for the glass background.
    assert.match(mainSource, /if \(vivo && \(vivoBackground === "glass" \|\| vivoBackground === "wallpaper"\)\)/)
    // The app's opacity fades the background layer and the cards, not the text.
    assert.match(mainSource, /visible: root\.vivo && !root\.hifi && root\.vivoBackground !== "none"\n\s+opacity: root\.backgroundOpacity/)
    assert.match(mainSource, /\(root\.vivo \? 0\.6 : 1\) \* root\.backgroundOpacity/)
})

test("Partículas e Ondas usam os shaders compilados", async () => {
    const background = await readFile(new URL("ui/VivoBackground.qml", base), "utf8")
    assert.match(configPage, /readonly property var backgroundValues: \["aurora", "glass", "particles", "waves", "none"\]/)
    assert.match(background, /fragmentShader: Qt\.resolvedUrl\("\.\.\/shaders\/particles\.frag\.qsb"\)/)
    assert.match(background, /fragmentShader: Qt\.resolvedUrl\("\.\.\/shaders\/waves\.frag\.qsb"\)/)
    for (const name of ["particles", "waves"])
        await readFile(new URL(`shaders/${name}.frag.qsb`, base))
})

test("com outra saída tocando, o topo mostra o ícone e o nome dela", () => {
    // Leve: the output's icon replaces the headset photo, its name the subtitle.
    assert.match(mainSource, /Kirigami\.Icon \{\n\s+visible: !root\.vivo && !root\.outputIsQuantum\n\s+source: root\.outputIcon/)
    assert.match(mainSource, /text: root\.outputIsQuantum \? "JBL Quantum 810 Wireless" : root\.outputLabel/)
    assert.match(mainSource, /text: "bateria do JBL Quantum 810"/)
    // Vivo: same in the ring, the path's last node, and the battery label.
    assert.match(vivoSource, /visible: !panel\.outputIsQuantum\n\s+width: parent\.width \* 0\.5\n\s+height: width\n\s+source: panel\.outputIcon/)
    assert.match(vivoSource, /panel\.outputIsQuantum \? "Fone" : "Saída"/)
    assert.match(vivoSource, /\(panel\.outputIsQuantum \? "" : "Fone "\)/)
    assert.match(mainSource, /soniccoreCommand\(\["output", "--format", "json"\]\)/)
})

test("a paleta do Vivo usa todas as cores das luzes, sem preto nem repetidas", async () => {
    const paletteSource = await readFile(new URL("ui/palette.js", base), "utf8")
    const Qt = { rgba: (r, g, b, a) => ({ r, g, b, a }) }
    const Palette = new Function("Qt", paletteSource.replace(".pragma library", "")
        + "\nreturn { build, at, fullBrightness }")(Qt)
    const wave = ["#37309a", "#9a3095", "#9a3033", "#9a8530", "#309a30"]

    assert.deepEqual(Palette.build(true, wave, wave, 5), wave)
    assert.deepEqual(Palette.build(true, "#FF0000", ["#000000", "00ff00", "bad"], 5),
        ["#ff0000", "#00ff00"])
    assert.deepEqual(Palette.build(false, wave, wave, 5), [])
    assert.deepEqual(Palette.build(true, null, null, 5), [])

    // Dimmed effect colors keep their hue at full brightness.
    assert.equal(Palette.fullBrightness("#120606"), "#ff5555")
    assert.equal(Palette.fullBrightness("#0E1113"), "#bce4ff")
    assert.equal(Palette.fullBrightness("#020202"), "#000000")
    assert.equal(Palette.fullBrightness("bad"), "")
    assert.equal(Palette.at([], 0.5, "fallback"), "fallback")
    assert.deepEqual(Palette.at(["#ff0000", "#0000ff"], 0.5, null), { r: 0.5, g: 0, b: 0.5, a: 1 })
    assert.deepEqual(Palette.at(wave, 1, null).g, 0x9a / 255)
    // The header, the spectrum and the background all read the palette.
    assert.match(mainSource, /lightPalette: root\.vivoPalette[\s\S]*lightPalette: root\.hifi \? \[String\(root\.vfdColor\)\] : root\.vivoPalette/)
    assert.doesNotMatch(vivoSource, /mix\(panel\.accentA, panel\.accentB/)
})

test("o tema Hi-Fi é um rack: madeira, alumínio escovado com as luzes, display verde e faders", async () => {
    const appSource = await readFile(new URL("../packaging/app/main.qml", import.meta.url), "utf8")
    assert.match(appSource, /text: "Hi-Fi"[\s\S]{0,400}onTriggered: settings\.theme = "hifi"/)
    assert.match(mainSource, /readonly property bool hifi: root\.configuration\.theme === "hifi"/)
    // The Vivo features (spectrum, light palette) stay on under the skin.
    assert.match(mainSource, /readonly property bool vivo: root\.configuration\.theme === "vivo" \|\| hifi/)
    assert.match(mainSource, /fragmentShader: Qt\.resolvedUrl\("\.\.\/shaders\/brushed\.frag\.qsb"\)/)
    assert.match(mainSource, /fragmentShader: Qt\.resolvedUrl\("\.\.\/shaders\/wood\.frag\.qsb"\)/)
    // The metal's sheen follows the lights and the bass.
    assert.match(mainSource, /property real pulse: root\.hifiPulse/)
    assert.match(mainSource, /Palette\.at\(root\.vivoPalette, 0\.5, "white"\)/)
    // Every slider (equalizer, 3D card, volume) becomes a fader.
    assert.equal((mainSource.match(/HifiFader \{ slider: /g) || []).length, 4)
    for (const name of ["brushed", "wood"])
        await readFile(new URL(`shaders/${name}.frag.qsb`, base))
})

test("no Hi-Fi as teclas, chaves e o seletor ganham pele de metal e voltam ao normal fora dele", () => {
    for (const skin of ["HifiKeySkin", "HifiSwitchSkin", "HifiComboSkin"])
        assert.match(mainSource, new RegExp(`component ${skin}: Item \\{[\\s\\S]{0,400}visible: root\\.hifi`))
    // The control's own look is hidden only while the theme is on.
    assert.match(mainSource, /Binding \{ target: key\.control\.background; property: "opacity"; value: root\.hifi \? 0 : 1 \}/)
    assert.match(mainSource, /restoreMode: Binding\.RestoreBindingOrValue/)
    assert.match(mainSource, /running: root\.hifi && root\.expanded\n\s+onTriggered: root\.applyHifiSkins\(expandedRepresentation\)/)
})

test("Tocando agora controla o player do Plasma e o volume geral do PC", () => {
    assert.match(mainSource, /import org\.kde\.plasma\.private\.mpris as Mpris/)
    assert.match(mainSource, /readonly property var outputSink: Volume\.PreferredDevice\.sink/)
    for (const call of ["Previous", "PlayPause", "Next"])
        assert.match(mainSource, new RegExp(`onClicked: root\\.mediaPlayer\\.${call}\\(\\)`))
    // 100% is PulseAudio's normal volume; never above it from here.
    assert.match(mainSource, /readonly property int normalVolume: 65536/)
    assert.match(mainSource, /Math\.max\(0, Math\.min\(100, percent\)\)/)
})
