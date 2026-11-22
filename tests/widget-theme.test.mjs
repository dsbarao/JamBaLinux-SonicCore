import { readFileSync } from "node:fs"
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const base = new URL("../packaging/app/contents/", import.meta.url)
const mainSource = await readFile(new URL("ui/SonicCore.qml", base), "utf8")
const vivoSource = await readFile(new URL("ui/VivoPanel.qml", base), "utf8")
// The app keeps the appearance settings and offers the themes and backgrounds.
const appMain = await readFile(new URL("../main.qml", base), "utf8")

function extract(source, name) {
    const match = source.match(new RegExp(`    function ${name}\\(([^)]*)\\) \\{([\\s\\S]*?)^    \\}\\n`, "m"))
    assert.notEqual(match, null, `${name} must exist`)
    return new Function(...match[1].split(",").map(arg => arg.trim()).filter(Boolean),
        `with (this) {${match[2]}}`)
}

test("o Vivo é o padrão do app e o seletor oferece Vivo, Hi-Fi e Leve", () => {
    assert.match(appMain, /property string theme: "vivo"/)
    for (const name of ["Vivo", "Hi-Fi", "Leve"])
        assert.match(appMain, new RegExp(`text: "${name}"\\n\\s+(tooltip: .*\\n\\s+)?checkable: true`))
    assert.match(mainSource, /readonly property bool vivo: root\.configuration\.theme === "vivo"/)
})

test("o Vivo só substitui o topo e só roda com o painel aberto", () => {
    assert.match(mainSource, /id: vfdWindow\n\s+visible: root\.vivo/)
    assert.match(mainSource, /VivoPanel \{\n\s+id: vivoPanel\n/)
    assert.match(mainSource, /running: root\.vivo && root\.expanded/)
    // The Leve header (image and big battery label) hides only in Vivo.
    assert.match(mainSource, /Image \{\n\s+visible: !root\.vivo && root\.outputIsQuantum\n\s+source: root\.equipmentImage/)
    assert.match(mainSource, /SonicControls\.Label \{\n\s+visible: !root\.vivo/)
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
    assert.match(mainSource, /readonly property bool usePcLighting: !useEngineColors && headsetConnected !== true && pcLighting !== null/)
    assert.match(mainSource, /: usePcLighting \? pcLighting\.colors\.map\(Palette\.fullBrightness\)/)
    assert.match(mainSource, /themeLogoColors: useComputerColors \? pcColors\.slice\(0, 5\)/)
    assert.match(mainSource, /themeRingColors: useComputerColors \? pcColors\.slice\(5, 10\)/)
    assert.match(mainSource, /vivoPalette: Palette\.build\(themeLightingEnabled, themeLogoColors,/)
    assert.match(mainSource, /"\/jambalinux-soniccore-openrgb\.json"/)
    const launcher = readFileSync(new URL("../packaging/app/launcher/main.cpp", import.meta.url), "utf8")
    assert.match(launcher, /qputenv\("QML_XHR_ALLOW_FILE_READ", "1"\)/)
})

test("o fundo do Vivo fica atrás de tudo, só no Vivo, e tem padrões seguros", () => {
    assert.match(appMain, /property string background: "aurora"/)
    assert.match(appMain, /property int backgroundIntensity: 1/)
    assert.match(appMain, /property bool backgroundReactive: true/)
    // The background is the first child of the popup, behind the controls.
    assert.match(mainSource, /fullRepresentation: Item \{[\s\S]{0,1600}VivoBackground \{[\s\S]{0,200}visible: root\.vivo && !root\.hifi && root\.vivoBackground !== "none"/)
    assert.match(mainSource, /running: root\.vivo && !root\.hifi && root\.expanded\n\s+bass: vivoPanel\.bass/)
    // The wallpaper is only looked up for the backgrounds that show it, or
    // for the PC lights' "Papel de parede" theme.
    assert.match(mainSource, /if \(\(vivo && \(vivoBackground === "glass" \|\| vivoBackground === "wallpaper"\)\)\n\s+\|\| pcLightsUses\("wallpaper"\)\)/)
    // The app's opacity fades the background layer and the cards, not the text.
    assert.match(mainSource, /visible: root\.vivo && !root\.hifi && root\.vivoBackground !== "none"\n\s+opacity: root\.backgroundOpacity/)
    assert.match(mainSource, /\(root\.vivo \? 0\.6 : 1\) \* root\.backgroundOpacity/)
})

test("Partículas e Ondas usam os shaders compilados", async () => {
    const background = await readFile(new URL("ui/VivoBackground.qml", base), "utf8")
    for (const value of ["aurora", "glass", "wallpaper", "particles", "waves", "none"])
        assert.match(appMain, new RegExp(`onTriggered: settings\\.background = "${value}"`))
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
    // Every slider (equalizer, 3D card, volume, the player's volume, the
    // PC lights' speed and brightness) becomes a fader.
    assert.equal((mainSource.match(/HifiFader \{ slider: /g) || []).length, 8)
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

test("Tocando agora usa o MPRIS e o volume do PipeWire, em qualquer ambiente", async () => {
    assert.match(mainSource, /Sonic\.MediaController \{ id: media \}/)
    assert.match(mainSource, /Sonic\.OutputVolume \{ id: outputVolume \}/)
    for (const call of ["previous", "playPause", "next"])
        assert.match(mainSource, new RegExp(`onClicked: media\\.${call}\\(\\)`))
    // Never above 100% from here (amplifying stays in the desktop's applet).
    assert.match(mainSource, /Math\.max\(0, Math\.min\(100, percent\)\)/)
    // No Plasma-only module is left in the interface.
    for (const name of ["SonicCore", "VivoPanel", "VivoBackground", "configBattery", "configOutput"]) {
        const source = await readFile(new URL(`ui/${name}.qml`, base), "utf8")
        assert.doesNotMatch(source, /import org\.kde\.(plasma|kcmutils)/, name)
    }
    assert.doesNotMatch(appMain, /import org\.kde\.(plasma|kcmutils)/)
})

test("o equalizador tem escala de nível ao lado das faixas em todos os temas", () => {
    // Plasma's controls get the marks drawn by the app; Hi-Fi has its own
    // fader scale and Qt's desktop style its tick marks.
    assert.match(mainSource, /model: !root\.hifi && Sonic\.App\.controls === "plasma" \? 9 : 0/)
    assert.match(mainSource, /index \/ 8 \* \(equalizerSlider\.availableHeight - handleSize\)/)
    assert.match(mainSource, /width: zero \? 6 : 4/)
})

test("o volume fica acima do Ganho e o Tocando agora tem a barra de tempo", () => {
    // Volume, then the equalizer body that starts with "Ganho".
    const volume = mainSource.indexOf("id: outputVolumeRow")
    const body = mainSource.indexOf("id: equalizerBody")
    const gain = mainSource.indexOf("id: equalizerGainRow")
    const profile = mainSource.indexOf("id: equalizerProfileSelector")
    assert.ok(volume > 0 && volume < body && body < gain && gain < profile)
    // The seek bar moves the track only on release.
    assert.match(mainSource, /if \(pressed\) return\n\s+media\.setPosition\(value\)/)
    const mediaTime = new Function("milliseconds",
        mainSource.match(/function mediaTime\(milliseconds\) \{([\s\S]*?)\n    \}/)[1])
    assert.equal(mediaTime(83000), "1:23")
    assert.equal(mediaTime(3723000), "1:02:03")
    assert.equal(mediaTime(-5), "0:00")
})

test("a Iluminação fica centrada e os segmentos 1 a 5 ficam lado a lado", () => {
    // An alignment is never joined to a condition: `Qt.AlignHCenter && x`
    // turns into true (= Qt.AlignLeft) and shows the control on the left.
    for (const source of [mainSource, vivoSource, appMain])
        assert.doesNotMatch(source, /Layout\.alignment: [^\n]*\n\s*(&&|\|\|)/)
    assert.match(mainSource,
        /Layout\.alignment: Qt\.AlignHCenter\n\s+visible: root\.openRgbControlsLighting \|\| root\.lightingHeld\n/)
    assert.match(mainSource,
        /Layout\.alignment: Qt\.AlignHCenter\n\s+visible: root\.openRgbLinking \|\| root\.openRgbCanLink\(\)\n/)
    // A key row counts only children with a width of their own: the five
    // segments have one, as tall as "Todos".
    const segments = mainSource.match(/id: allSegmentsKey[\s\S]*?Repeater \{\n\s+model: 5\n([\s\S]*?)MouseArea/)
    assert.notEqual(segments, null, "the segment keys must follow \"Todos\"")
    assert.match(segments[1], /implicitWidth: \d+/)
    assert.match(segments[1], /implicitHeight: allSegmentsKey\.implicitHeight/)
})

test("a Opacidade deixa ver o desktop com qualquer fundo, e o Vidro desfoca o que está atrás", async () => {
    // The window's color goes to the compositor premultiplied by the
    // opacity; a plain color would be added in full over what is behind.
    assert.match(appMain, /color: Qt\.rgba\(Kirigami\.Theme\.backgroundColor\.r \* clearOpacity,\n\s+Kirigami\.Theme\.backgroundColor\.g \* clearOpacity,\n\s+Kirigami\.Theme\.backgroundColor\.b \* clearOpacity, clearOpacity\)/)
    // "Vidro" and "Papel de parede" draw their own picture, faded as one
    // layer: the window under them stays clear, or the two would add up.
    assert.match(appMain, /readonly property real clearOpacity: pictureBackground \? 0 : settings\.windowOpacity/)
    assert.match(appMain, /readonly property bool pictureBackground: settings\.theme === "vivo"\n\s+&& \(settings\.background === "glass" \|\| settings\.background === "wallpaper"\)/)
    const background = await readFile(new URL("ui/VivoBackground.qml", base), "utf8")
    assert.match(background, /id: canvas\n\s+anchors\.fill: parent\n\s+layer\.enabled: background\.style !== "none"/)
    assert.match(background, /Rectangle \{\n\s+visible: background\.style === "glass" \|\| background\.style === "wallpaper"\n\s+anchors\.fill: parent\n\s+color: Kirigami\.Theme\.backgroundColor\n\s+\}\n\n\s+Image \{/)
    // Below 100%, "Vidro" asks the compositor to blur what is behind.
    assert.match(appMain, /readonly property bool wantsBlur: visible && settings\.theme === "vivo"\n\s+&& settings\.background === "glass" && settings\.windowOpacity < 0\.999/)
    assert.match(appMain, /blurred = Sonic\.App\.setBlurBehind\(window, wantsBlur\)/)
    // And the window has an alpha channel on every desktop (X11 included).
    const launcher = await readFile(new URL("../launcher/main.cpp", base), "utf8")
    assert.match(launcher, /QQuickWindow::setDefaultAlphaBuffer\(true\);[\s\S]*QApplication app\(/)
})

test("Tocando agora tem o volume do próprio player, em pé à direita", () => {
    assert.match(mainSource, /Sonic\.PlayerVolume \{\n\s+id: playerVolume\n\s+pid: media\.pid\n\s+names: media\.hasPlayer \? \[media\.iconName, media\.identity\] : \[\]/)
    const card = mainSource.slice(mainSource.indexOf('title: "Tocando agora"'), mainSource.indexOf("// ---- Rádio ----"))
    // The player's column first, the volume column after it (on the right).
    assert.ok(card.indexOf("id: mediaPositionSlider") < card.indexOf("id: playerVolumeColumn"))
    assert.match(card, /visible: media\.hasPlayer && playerVolume\.available/)
    assert.match(card, /orientation: Qt\.Vertical\n[\s\S]{0,300}onMoved: playerVolume\.setPercent\(Math\.round\(value\)\)/)
    assert.match(card, /onTapped: playerVolume\.setMuted\(!playerVolume\.muted\)/)
})

test("com o Sonic Core no comando das luzes do PC, os efeitos seguem as cores que ele desenha", () => {
    // Its engine publishes ten colors of the effect it draws; they lead,
    // headset on or off, while it is in charge.
    assert.match(mainSource, /readonly property bool useEngineColors: pcLightsActive && pcLightsStatus\?\.in_charge === true\n\s+&& pcLightsFrame !== null/)
    assert.match(mainSource, /pcColors: useEngineColors \? pcLightsFrame\.colors\.map\(Palette\.fullBrightness\)/)
    assert.match(mainSource, /"\/jambalinux-soniccore-pc-lights-frame\.json"/)
    assert.match(mainSource, /running: root\.expanded && root\.vivo && root\.pcLightsActive\n\s+onTriggered: root\.readPcLightsFrame\(\)/)
})
