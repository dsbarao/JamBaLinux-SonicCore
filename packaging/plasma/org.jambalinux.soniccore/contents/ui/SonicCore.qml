import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQml.Models

import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.plasma5support as Plasma5Support
import org.kde.plasma.workspace.dbus as DBus

import "palette.js" as Palette

// The whole SonicCore interface, shared by the Plasma widget (main.qml)
// and the standalone app (packaging/app/main.qml). The host provides the
// settings in `configuration` and says when the panel is on screen with
// `expanded`; it decides what a click on the compact view does.
Item {
    id: root

    // One section of the panel. Every card is always shown; when what it
    // holds changes (lights switched off, a warning appears) it grows or
    // shrinks smoothly, and it fades while its controls are unavailable.
    component SectionCard: Rectangle {
        id: card
        property string title
        property string subtitle
        default property alias content: cardBody.data

        Layout.fillWidth: true
        implicitHeight: cardBody.implicitHeight + Kirigami.Units.largeSpacing * 2
        radius: Kirigami.Units.cornerRadius * 2
        color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g,
            Kirigami.Theme.backgroundColor.b, root.vivo ? 0.6 : 1)
        border.width: 1
        border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g,
            Kirigami.Theme.textColor.b, 0.15)
        clip: true
        opacity: enabled ? 1 : 0.45

        Behavior on implicitHeight {
            NumberAnimation { duration: Kirigami.Units.longDuration; easing.type: Easing.OutCubic }
        }
        Behavior on opacity {
            NumberAnimation { duration: Kirigami.Units.longDuration }
        }

        ColumnLayout {
            id: cardBody
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.smallSpacing

            PlasmaComponents.Label {
                Layout.alignment: Qt.AlignHCenter
                visible: text.length > 0
                text: card.title
                font.bold: true
            }

            PlasmaComponents.Label {
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumWidth: cardBody.width
                visible: text.length > 0
                text: card.subtitle
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                opacity: 0.7
            }
        }
    }

    property var configuration: ({})
    property bool expanded: false
    signal toggleRequested()

    // Keep the unexpanded home reference in one place. The non-interactive
    // shell in soniccoreCommand expands it once when it starts the CLI.
    readonly property string soniccoreBinary: "$HOME/.cargo/bin/soniccore"
    readonly property string command: soniccoreCommand(["status", "--format", "json"])
    readonly property string cachedCommand: soniccoreCommand(["cached-status", "--format", "json"])
    readonly property string equalizerCommand: soniccoreCommand(["equalizer", "status", "--format", "json"])
    readonly property string spatialCommand: soniccoreCommand(["spatial", "status", "--format", "json"])
    readonly property string spatialEnableCommand: soniccoreCommand(["spatial", "enable"])
    readonly property string spatialDisableCommand: soniccoreCommand(["spatial", "disable"])
    readonly property url equipmentImage: Qt.resolvedUrl("../images/jbl-quantum-810.png")
    property int batteryPercent: -1
    property bool charging: false
    property string rawFeature: ""
    property string errorMessage: ""
    property string actionMessage: ""
    property bool actionFailed: false
    // "Vincular com OpenRGB": `soniccore openrgb` status, refreshed when the
    // lights section opens, and the result of the last link attempt.
    property var openRgbStatus: null
    property bool openRgbLinking: false
    property string openRgbMessage: ""
    readonly property string openRgbLinkCommand: soniccoreCommand(["openrgb", "link", "--format", "json"])
    property string ambientMode: "unknown"
    property var headsetConnected: null
    property string microphoneState: "unknown"
    property string sidetoneLevel: "unknown"
    property var lightingEnabled: null
    // Lighting profile fields are nullable. In particular, a partial HID
    // write makes the affected zone unknown; do not turn that into defaults.
    property var lightingColor: null
    property var logoColor: null
    property var ringColor: null
    property var logoColors: null
    property var ringColors: null
    property var logoEffect: null
    property var ringEffect: null
    property var logoSpeed: null
    property var ringSpeed: null
    property string lightingTarget: "both"
    property int lightingSegment: -1
    property real pickerHue: 0
    property real pickerSaturation: 1
    property real pickerValue: 1
    readonly property color pickerColor: Qt.hsva(pickerHue, pickerSaturation, pickerValue, 1)
    property int gameChatValue: -1
    property bool controlBusy: false
    // Who wrote the lighting last ("soniccore" or "openrgb"), when, and
    // whether SonicCore holds it (OpenRGB writes are then refused).
    property string lightingSource: ""
    property real lightingUpdatedAtMs: 0
    property bool lightingHeld: false
    // Battery economy: why it switched the lights off ("low-battery",
    // "idle" or empty) and the measured estimate of hours left.
    property string powerSaving: ""
    property real hoursLeft: -1
    readonly property string hoursLeftText: hoursLeft > 0 && !charging
        ? (hoursLeft >= 1 ? `~${Math.round(hoursLeft)} h restantes` : `~${Math.max(1, Math.round(hoursLeft * 60))} min restantes`)
        : ""
    property real nowMs: Date.now()
    // OpenRGB wrote within the last few seconds and nobody took the lights
    // back: changes made here would be overwritten by its next frame.
    readonly property bool openRgbControlsLighting: !lightingHeld
        && lightingSource === "openrgb"
        && nowMs - lightingUpdatedAtMs < 5000
    readonly property bool lightingWritable: deviceAvailable && !openRgbControlsLighting
    // Live color while dragging the picker: only the latest value waits, and
    // it is sent when the running command finishes and the throttle allows.
    property string pendingLightingFeature: ""
    property string pendingLightingColor: ""
    property string lastLiveLightingKey: ""
    property bool updating: false
    // Set when a refresh is requested while another is running, so a burst
    // of daemon signals is never reduced to the first, stale read.
    property bool refreshPending: false
    property bool equalizerUpdating: false
    property bool equalizerBusy: false
    property int equalizerVisualRevision: 0
    property bool equalizerPipeWireActive: false
    property var equalizerServiceActive: null
    // "Som natural": the DSP runs flat; the saved bands stay as they are.
    property bool equalizerBypassed: false
    // Listening mode of the 3D sound and its fine controls (dB / share).
    property string spatialListening: "game"
    property var spatialSound: ({ "intensity": 1, "dialogue_db": 0, "bass_db": 0, "surround_db": 0 })
    property var equalizerTargetConnected: null
    property var equalizerDefaultSafe: null
    property string equalizerTarget: ""
    property string equalizerStatusError: ""
    property string equalizerActionError: ""
    property string pendingEqualizerAction: ""
    // Latest requested gain per frequency (Hz -> dB) not sent yet. A slider
    // moved while another `set` runs lands here and is sent when that command
    // finishes: a change is replaced by a newer one for the same band, never
    // dropped.
    property var pendingEqualizerBands: ({})
    property string equalizerSetCommand: ""
    property string equalizerActiveProfileId: ""
    property int equalizerProfileRevision: 0
    property bool equalizerCreateVisible: false
    property bool equalizerRenameVisible: false
    property bool equalizerDeleteConfirmation: false
    property string equalizerProfileNameDraft: ""
    property bool spatialUpdating: false
    property bool spatialBusy: false
    property bool spatialEnabled: false
    property string spatialMode: "off"
    property bool spatialReady: false
    property bool spatialDatasetValid: false
    property bool spatialActive: false
    property string spatialProcessingMode: "bypass"
    readonly property bool spatialMixPending: spatialActive
        && ((spatialEnabled && spatialProcessingMode !== "binaural")
            || (!spatialEnabled && spatialProcessingMode !== "bypass"))
    property bool spatialServiceActive: false
    property string spatialError: ""
    property string spatialActionMessage: ""
    readonly property var equalizerFrequencies: [
        31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000
    ]
    // Immutable built-in presets, in the CLI's own order. The equalizer status
    // JSON reports only the user-owned library, so the factory catalog stays
    // declared here; it is never written back and carries no band data.
    readonly property var equalizerFactoryProfiles: [
        { "label": "Flat", "profileId": "flat" },
        { "label": "Bass Boost", "profileId": "bass-boost" },
        { "label": "Cinematic", "profileId": "cinematic" },
        { "label": "FPS", "profileId": "fps" },
        { "label": "MOBA", "profileId": "moba" },
        { "label": "RPG", "profileId": "rpg" },
        { "label": "Apex Legends", "profileId": "apex-legends" },
        { "label": "CS2", "profileId": "cs2" },
        { "label": "Dota 2", "profileId": "dota-2" },
        { "label": "Fortnite", "profileId": "fortnite" },
        { "label": "GTA 5", "profileId": "gta-5" },
        { "label": "LoL", "profileId": "lol" },
        { "label": "PUBG", "profileId": "pubg" },
        { "label": "WoW", "profileId": "wow" },
        { "label": "Escape from Tarkov", "profileId": "escape-from-tarkov" },
        { "label": "LN3 Immersion", "profileId": "ln3-immersion" },
        { "label": "LN3 Thrill", "profileId": "ln3-thrill" }
    ]
    // A headset reported off keeps its last battery reading on screen (in
    // grey), but nothing can be written to it.
    // "leve" keeps the original interface; "vivo" adds the live header.
    readonly property bool vivo: root.configuration.theme === "vivo"
    readonly property string vivoBackground: root.configuration.background || "aurora"
    // Where the equalizer is playing now (it follows the system default in
    // "auto"). Another output shows its own icon and name in the header.
    property bool outputIsQuantum: true
    property string outputIcon: "audio-headphones"
    property string outputLabel: ""
    // The desktop wallpaper for the "glass" background, looked up on open.
    property url wallpaperUrl
    // The lights' own colors tint the Vivo theme; black or unknown zones
    // fall back to the system accent.
    readonly property color vivoAccentA: vivoAccent(Array.isArray(logoColors) ? logoColors[0] : logoColor)
    readonly property color vivoAccentB: vivoAccent(Array.isArray(ringColors) ? ringColors[0] : ringColor)
    // Every light color (all segments of both zones), so the theme follows
    // multi-color effects too, not just the first segment.
    readonly property var vivoPalette: Palette.build(lightingEnabled, logoColors ?? logoColor,
                                                     ringColors ?? ringColor, 5)
    readonly property bool deviceAvailable: batteryPercent >= 0 && headsetConnected !== false
    readonly property bool daemonAvailable: daemonWatcher.registered
    readonly property color batteryColor: headsetConnected === false
        ? Kirigami.Theme.disabledTextColor
        : batteryPercent < 0
        ? Kirigami.Theme.disabledTextColor
        : charging
        ? "#22d3ee"
        : batteryPercent >= 60
        ? "#35c759"
        : batteryPercent >= 30 ? "#f5c542" : "#ff453a"

    ListModel {
        id: equalizerBandModel
    }

    ListModel {
        // A transient view of the catalog reported by the equalizer status
        // JSON, never a second persistent store of custom profiles.
        id: equalizerProfileModel
    }

    function refresh() {
        if (updating) {
            refreshPending = true
            return
        }
        updating = true
        errorMessage = ""
        cachedState.connectSource(cachedCommand)
    }

    function vivoAccent(value) {
        if (lightingEnabled === false || typeof value !== "string") return Kirigami.Theme.highlightColor
        const hex = String(normalizeColor(value)).toLowerCase()
        return /^#[0-9a-f]{6}$/.test(hex) && hex !== "#000000" ? hex : Kirigami.Theme.highlightColor
    }

    function finishRefresh(data) {
        updating = false
        applyResult(data)
        if (refreshPending) {
            refreshPending = false
            refreshAfterControl.restart()
        }
    }

    function runControl(feature, value) {
        if (controlBusy) return
        controlBusy = true
        clearAction.stop()
        actionFailed = false
        actionMessage = "Aplicando…"
        const controlCommand = soniccoreCommand(["set", feature, value])
        executable.connectSource(controlCommand)
    }

    function refreshOpenRgb() {
        openRgbSource.connectSource(soniccoreCommand(["openrgb", "--format", "json"]))
    }

    function linkOpenRgb() {
        if (openRgbLinking) return
        openRgbLinking = true
        openRgbMessage = ""
        openRgbSource.connectSource(openRgbLinkCommand)
    }

    function openRgbCanLink() {
        const status = openRgbStatus
        return !!status && status.openrgb_installed === true && status.supported === true
            && (status.missing_tools ?? []).length === 0
            && !(status.plugin_installed === true && status.plugin_current !== false)
    }

    function openRgbSummary() {
        const status = openRgbStatus
        if (openRgbLinking)
            return "Vinculando: baixando os cabeçalhos do OpenRGB e compilando o plugin (leva alguns segundos)…"
        if (openRgbMessage.length > 0) return openRgbMessage
        if (!status) return ""
        if (status.openrgb_installed !== true)
            return "Instale o OpenRGB para controlar as luzes do fone junto com seus outros dispositivos RGB."
        if (status.supported !== true)
            return `O OpenRGB ${status.openrgb_version ?? "?"} ainda não é suportado (precisa da versão ${(status.supported_versions ?? []).join(", ")}).`
        if (status.plugin_installed === true && status.plugin_enabled === false)
            return "O plugin está instalado, mas desativado no OpenRGB (Configurações → Plugins)."
        if (status.plugin_installed === true && status.plugin_current !== false)
            return "Vinculado ao OpenRGB."
        if ((status.missing_tools ?? []).length > 0)
            return `Para vincular, instale antes: ${status.missing_tools.join(", ")}.`
        if (status.plugin_installed === true)
            return "Há uma versão nova do plugin do OpenRGB."
        return "Vincule para o OpenRGB mostrar o fone como um dispositivo dele."
    }

    function applyOpenRgbResult(sourceName, data) {
        let result = null
        try {
            result = JSON.parse(String(data.stdout ?? ""))
        } catch (error) {
        }
        if (sourceName === openRgbLinkCommand) {
            openRgbLinking = false
            if (result && result.ok === true) {
                openRgbMessage = result.openrgb_running === true
                    ? "Vinculado! Feche e abra o OpenRGB para ele carregar o plugin."
                    : "Vinculado! Abra o OpenRGB: o fone aparece na lista de dispositivos."
            } else {
                const detail = result?.message ?? String(data.stderr ?? "").trim()
                openRgbMessage = `Não deu para vincular: ${detail || "erro desconhecido"}`
            }
            refreshOpenRgb()
            return
        }
        if (result && result.schema === 1) openRgbStatus = result
    }

    // Every section is on screen whenever the panel is: load what each
    // one shows when it appears.
    onExpandedChanged: {
        if (!expanded) {
            closeEqualizerProfileEditors()
            return
        }
        if (!openRgbLinking) openRgbMessage = ""
        refreshOpenRgb()
        loadPicker()
        refreshEqualizer()
        // The wallpaper can change at any time; look it up each time.
        if (vivo && vivoBackground === "glass")
            wallpaperSource.connectSource(soniccoreCommand(["wallpaper", "--format", "json"]))
    }

    function runLightingHold(held) {
        if (controlBusy) return
        controlBusy = true
        clearAction.stop()
        actionFailed = false
        actionMessage = held ? "Assumindo as luzes…" : "Devolvendo as luzes ao OpenRGB…"
        executable.connectSource(soniccoreCommand(["lighting-hold", held ? "on" : "off"]))
    }

    function queueLightingColor() {
        if (!lightingWritable || !canApplyLightingColor()) return
        pendingLightingFeature = lightingFeature()
        pendingLightingColor = pickerHex()
        if (!lightingColorThrottle.running) flushLightingColor()
    }

    function flushLightingColor() {
        if (pendingLightingColor === "" || controlBusy) return
        const feature = pendingLightingFeature
        const value = pendingLightingColor
        pendingLightingFeature = ""
        pendingLightingColor = ""
        // Each color write sends a full lighting profile to the headset, so
        // the same value is never sent twice in a row.
        const key = `${feature} ${value}`
        if (key === lastLiveLightingKey) return
        lastLiveLightingKey = key
        controlBusy = true
        executable.connectSource(soniccoreCommand(["set", feature, value]))
        lightingColorThrottle.restart()
    }

    function refreshEqualizer() {
        if (equalizerUpdating) return
        equalizerUpdating = true
        equalizerExecutable.connectSource(equalizerCommand)
    }

    function refreshSpatial() {
        if (spatialUpdating || spatialBusy) return
        spatialUpdating = true
        spatialExecutable.connectSource(spatialCommand)
    }

    function setSpatialEnabled(enabled) {
        if (spatialBusy || spatialUpdating) {
            return
        }
        // The CLI repeats this check before persisting the gate. Keeping the
        // action unavailable here avoids a misleading optimistic toggle.
        if (enabled && !spatialReady) {
            spatialError = "A capacidade espacial não está pronta; instale manualmente um dataset HRIR válido e execute a pré-verificação."
            return
        }
        spatialBusy = true
        spatialActionMessage = enabled ? "Ativando o mix binaural…" : "Ativando bypass sem interromper a reprodução…"
        spatialControlExecutable.connectSource(enabled ? spatialEnableCommand : spatialDisableCommand)
    }

    // Several CLI steps in one shell, stopping at the first failure.
    function soniccoreSequence(steps) {
        const program = steps.map(step => [soniccoreBinary].concat(step.map(shellQuote)).join(" ")).join(" && ")
        return "/bin/sh -c " + shellQuote(program)
    }

    // Natural switches the 3D off; a named mode sets its levels and makes
    // sure the 3D is on.
    function selectListeningMode(mode) {
        if (spatialBusy || spatialUpdating) return
        if (mode === "natural") {
            if (spatialEnabled) setSpatialEnabled(false)
            return
        }
        if (!spatialEnabled && !spatialReady) return
        spatialBusy = true
        spatialActionMessage = "Aplicando o modo…"
        const steps = [["spatial", "sound", mode]]
        if (!spatialEnabled) steps.push(["spatial", "enable"])
        spatialControlExecutable.connectSource(soniccoreSequence(steps))
    }

    function setSpatialSoundControl(control, value) {
        if (spatialBusy) return
        spatialBusy = true
        spatialActionMessage = "Ajustando…"
        spatialControlExecutable.connectSource(soniccoreCommand(
            ["spatial", "sound", "set", control, String(Math.round(value * 100) / 100)]))
    }

    function listeningLabel(mode) {
        if (mode === "game") return "Jogo"
        if (mode === "cinema") return "Cinema"
        if (mode === "music") return "Música"
        return "Personalizado"
    }

    function spatialStatusText() {
        if (root.spatialBusy)
            return root.spatialActionMessage
        if (!root.spatialDatasetValid)
            return "Indisponível neste computador"
        if (!root.spatialEnabled && root.spatialActive && root.spatialProcessingMode !== "bypass")
            return "Desligando…"
        if (!root.spatialEnabled)
            return "Desligado"
        if (root.spatialActive && root.spatialProcessingMode === "binaural")
            return `Ligado · ${root.listeningLabel(root.spatialListening)}`
        if (root.spatialError.length > 0)
            return "Não foi possível ligar"
        return "Ligando…"
    }

    function setEqualizerBand(frequencyHz, gainDb) {
        const normalizedGain = Math.round(Number(gainDb) * 10) / 10
        const pending = Object.assign({}, pendingEqualizerBands)
        pending[String(frequencyHz)] = normalizedGain
        pendingEqualizerBands = pending
        equalizerFullRefresh.stop()
        sendNextEqualizerBand()
    }

    function sendNextEqualizerBand() {
        if (equalizerBusy) return
        const frequencies = Object.keys(pendingEqualizerBands)
        if (frequencies.length === 0) return
        const frequency = frequencies[0]
        const gain = pendingEqualizerBands[frequency]
        const pending = Object.assign({}, pendingEqualizerBands)
        delete pending[frequency]
        pendingEqualizerBands = pending
        equalizerBusy = true
        pendingEqualizerAction = "set"
        equalizerActionError = ""
        equalizerSetCommand = equalizerCliCommand(
            ["set", "--format", "json", frequency, String(gain)])
        equalizerExecutable.connectSource(equalizerSetCommand)
    }

    // Handles follow the DSP's confirmed gains, except bands the user already
    // moved again: those keep the newer, still queued value.
    function showConfirmedEqualizerBands(bands) {
        const shown = []
        for (let index = 0; index < bands.length; ++index) {
            const frequency = String(bands[index].frequency_hz)
            shown.push({
                "frequency_hz": bands[index].frequency_hz,
                "gain_db": pendingEqualizerBands[frequency] ?? bands[index].gain_db
            })
        }
        replaceEqualizerBands(shown)
    }

    function finishEqualizerSet(exitCode, stdout, stderr) {
        equalizerBusy = false
        pendingEqualizerAction = ""
        let result = null
        try {
            result = JSON.parse(stdout)
        } catch (error) {
            result = null
        }
        const bands = result && Array.isArray(result.applied_bands) ? result.applied_bands : []
        if (bands.length === 10) {
            showConfirmedEqualizerBands(bands)
            equalizerActiveProfileId = result.active_profile_id === null
                || result.active_profile_id === undefined ? "" : String(result.active_profile_id)
        }
        const failed = exitCode !== 0 || Boolean(result && result.error)
        if (failed) {
            // The CLI answers a rejected change with the previous, still
            // applied profile: the handle goes back to the confirmed value.
            equalizerActionError = String(result?.error ?? "") || stderr
                || equalizerActionFailureMessage("set")
            equalizerVisualRevision += 1
        } else {
            equalizerActionError = ""
        }
        if (Object.keys(pendingEqualizerBands).length > 0) {
            sendNextEqualizerBand()
        } else if (failed || bands.length !== 10) {
            refreshEqualizer()
        } else {
            equalizerFullRefresh.restart()
        }
    }

    function shellQuote(value) {
        // POSIX single-quoting: the only character that cannot appear inside a
        // single-quoted word is the quote itself, which is closed, escaped and
        // reopened. Everything else stays literal data for the shell.
        return "'" + String(value).replace(/'/g, "'\"'\"'") + "'"
    }

    function soniccoreCommand(cliArguments) {
        // Quote every argument before joining the command. User-entered
        // profile names, control values, and future arguments remain data even
        // when they contain quotes, spaces, or shell metacharacters.
        const quotedArguments = cliArguments.map(shellQuote).join(" ")
        const program = soniccoreBinary
            + (quotedArguments.length > 0 ? " " + quotedArguments : "")
        // A plain, non-interactive shell avoids sourcing login profiles on
        // every widget poll while still expanding soniccoreBinary's $HOME.
        return "/bin/sh -c " + shellQuote(program)
    }

    function equalizerCliCommand(cliArguments) {
        return soniccoreCommand(["equalizer"].concat(cliArguments))
    }

    function startEqualizerAction(action, cliArguments) {
        if (equalizerBusy || equalizerUpdating) return false
        equalizerBusy = true
        pendingEqualizerAction = action
        equalizerActionError = ""
        equalizerExecutable.connectSource(equalizerCliCommand(cliArguments))
        return true
    }

    function applyEqualizerProfile(profileId, factory) {
        // Factory presets keep their dedicated verb; `profile apply` is
        // restricted to the user-owned library by the CLI itself.
        startEqualizerAction(factory ? "preset" : "apply", factory
            ? ["preset", profileId]
            : ["profile", "apply", profileId])
    }

    function equalizerProfileIndex(profileId) {
        // Reading the revision makes bindings that call this helper depend on
        // the catalog rebuild: list model contents alone notify nothing.
        const id = String(profileId)
        if (equalizerProfileRevision < 0 || id.length === 0) return -1
        for (let index = 0; index < equalizerProfileModel.count; ++index) {
            if (equalizerProfileModel.get(index).profileId === id)
                return index
        }
        return -1
    }

    function selectedEqualizerProfile() {
        const index = equalizerProfileIndex(equalizerActiveProfileId)
        return index >= 0 ? equalizerProfileModel.get(index) : null
    }

    function selectedEqualizerProfileIsCustom() {
        const profile = selectedEqualizerProfile()
        return profile !== null && profile.factory === false
    }

    function equalizerSelectionLabel() {
        // Derived from the canonical selection instead of the combo box index,
        // which the control resets on its own whenever the model is rebuilt.
        const profile = selectedEqualizerProfile()
        if (profile !== null) return String(profile.label)
        // Before the first status response no catalog is known yet, so manual
        // settings must not be announced for an unread selection.
        return equalizerProfileRevision === 0 ? "Carregando…" : "Ajustes manuais"
    }

    function createEqualizerProfile() {
        if (equalizerProfileNameDraft.trim().length === 0) return
        startEqualizerAction("create",
            ["profile", "create", equalizerProfileNameDraft.trim()])
    }

    function renameEqualizerProfile() {
        const profile = selectedEqualizerProfile()
        if (profile === null || profile.factory) return
        if (equalizerProfileNameDraft.trim().length === 0) return
        startEqualizerAction("rename",
            ["profile", "rename", profile.profileId,
             equalizerProfileNameDraft.trim()])
    }

    function updateEqualizerProfile() {
        const profile = selectedEqualizerProfile()
        if (profile === null || profile.factory) return
        startEqualizerAction("update", ["profile", "update", profile.profileId])
    }

    function deleteEqualizerProfile() {
        const profile = selectedEqualizerProfile()
        if (profile === null || profile.factory) return
        startEqualizerAction("delete", ["profile", "delete", profile.profileId])
    }

    function equalizerActionMessage(action) {
        if (action === "reset") return "Bandas zeradas"
        if (action === "bypass-on") return "Som natural: equalizador desligado"
        if (action === "bypass-off") return "Equalizador ligado"
        if (action === "preset" || action === "apply")
            return "Perfil de equalização aplicado"
        if (action === "create") return "Perfil personalizado criado"
        if (action === "update") return "Perfil personalizado atualizado"
        if (action === "rename") return "Perfil personalizado renomeado"
        if (action === "delete") return "Perfil personalizado excluído"
        return "Equalizador atualizado"
    }

    function equalizerActionFailureMessage(action) {
        if (action === "create") return "Não foi possível criar o perfil personalizado"
        if (action === "update") return "Não foi possível atualizar o perfil personalizado"
        if (action === "rename") return "Não foi possível renomear o perfil personalizado"
        if (action === "delete") return "Não foi possível excluir o perfil personalizado"
        if (action === "preset" || action === "apply")
            return "Não foi possível aplicar o perfil de equalização"
        return "Não foi possível atualizar o equalizador"
    }

    function submitEqualizerProfileName() {
        if (equalizerCreateVisible) createEqualizerProfile()
        else if (equalizerRenameVisible) renameEqualizerProfile()
    }

    function closeEqualizerProfileEditors() {
        equalizerCreateVisible = false
        equalizerRenameVisible = false
        equalizerDeleteConfirmation = false
        equalizerProfileNameDraft = ""
    }

    function resetEqualizer() {
        startEqualizerAction("reset", ["reset"])
    }

    function setEqualizerBypassed(bypassed) {
        startEqualizerAction(bypassed ? "bypass-on" : "bypass-off", ["bypass", bypassed ? "on" : "off"])
    }

    function replaceEqualizerBands(bands) {
        const gainsByFrequency = {}
        for (let index = 0; index < bands.length; ++index) {
            const frequency = Number(bands[index].frequency_hz)
            const gain = Number(bands[index].gain_db)
            if (Number.isFinite(frequency) && Number.isFinite(gain))
                gainsByFrequency[frequency] = gain
        }

        // Update rows in place: clear()+append() destroyed and recreated every
        // slider delegate, so the handle being dragged vanished mid-drag each
        // time a `set` answered (26/09: "the knob keeps sticking").
        if (equalizerBandModel.count !== equalizerFrequencies.length) {
            equalizerBandModel.clear()
            for (let index = 0; index < equalizerFrequencies.length; ++index)
                equalizerBandModel.append({ "frequencyHz": equalizerFrequencies[index], "gainDb": 0 })
        }
        for (let index = 0; index < equalizerFrequencies.length; ++index) {
            const gain = Number(gainsByFrequency[equalizerFrequencies[index]] ?? 0)
            if (equalizerBandModel.get(index).gainDb !== gain)
                equalizerBandModel.setProperty(index, "gainDb", gain)
        }
        equalizerVisualRevision += 1
    }

    function isFactoryEqualizerProfile(profileId) {
        for (let index = 0; index < equalizerFactoryProfiles.length; ++index) {
            if (equalizerFactoryProfiles[index].profileId === profileId)
                return true
        }
        return false
    }

    function equalizerCatalogMatchesModel(catalog) {
        const offset = equalizerFactoryProfiles.length
        if (equalizerProfileModel.count !== offset + catalog.length) return false
        for (let index = 0; index < catalog.length; ++index) {
            const shown = equalizerProfileModel.get(offset + index)
            if (shown.profileId !== catalog[index].profileId
                || shown.label !== catalog[index].label)
                return false
        }
        return true
    }

    function replaceEqualizerProfiles(customProfiles) {
        // The custom library is owned by the CLI; this model is only a view of
        // the catalog that the last status response reported.
        const catalog = []
        for (let index = 0; index < customProfiles.length; ++index) {
            const custom = customProfiles[index] ?? {}
            const id = String(custom.id ?? "")
            const name = String(custom.name ?? "")
            if (id.length > 0 && name.length > 0 && !isFactoryEqualizerProfile(id))
                catalog.push({ "label": name, "profileId": id, "factory": false })
        }

        // Band edits refresh the status too, and rebuilding an unchanged
        // catalog would reset the selector on every committed slider move.
        if (equalizerCatalogMatchesModel(catalog)) return

        equalizerProfileModel.clear()
        for (let index = 0; index < equalizerFactoryProfiles.length; ++index) {
            const factory = equalizerFactoryProfiles[index]
            equalizerProfileModel.append({
                "label": factory.label, "profileId": factory.profileId,
                "factory": true
            })
        }
        for (let index = 0; index < catalog.length; ++index)
            equalizerProfileModel.append(catalog[index])

        // A revision forces the selector to resynchronize even when the
        // canonical selection itself did not change.
        equalizerProfileRevision += 1
    }

    function equalizerProblemText() {
        if (equalizerDefaultSafe === false) {
            return "A saída virtual não pode ser a saída padrão. A restauração automática para a saída física está em andamento."
        }
        if (equalizerActionError.length > 0)
            return equalizerActionError
        if (equalizerStatusError.length > 0)
            return equalizerStatusError
        if (equalizerTargetConnected !== false && equalizerServiceActive === false) {
            return "O serviço automático do equalizador está inativo. Execute tools/install-user.sh novamente e confira o serviço do usuário."
        }
        return ""
    }

    function equalizerFrequencyLabel(frequencyHz) {
        return frequencyHz >= 1000 ? `${frequencyHz / 1000} kHz` : `${frequencyHz} Hz`
    }

    function equalizerCompactFrequencyLabel(frequencyHz) {
        return frequencyHz >= 1000 ? `${frequencyHz / 1000}k` : `${frequencyHz}`
    }

    function ambientLabel(value) {
        if (value === "off") return "Desligado"
        if (value === "anc") return "ANC"
        if (value === "talkthru") return "TalkThru"
        return "Aguardando estado"
    }

    function sidetoneLabel(value) {
        if (value === "off") return "Desligado"
        if (value === "low") return "Baixo"
        if (value === "medium") return "Médio"
        if (value === "high") return "Alto"
        return "Aguardando estado"
    }

    function selectedLightingColor() {
        if (lightingSegment >= 0) {
            const colors = lightingTarget === "ring" ? ringColors : logoColors
            return Array.isArray(colors) ? colors[lightingSegment] ?? null : null
        }
        if (lightingTarget === "logo") return logoColor
        if (lightingTarget === "ring") return ringColor
        return lightingColor
    }

    function lightingFeature() {
        // A synchronized color is the only recovery command that does not
        // depend on a cached profile. It takes precedence over an old segment
        // selection retained while the segment picker is hidden.
        if (lightingTarget === "both" && !lightingTargetProfileKnown()) return "color"
        if (lightingSegment >= 0) {
            const prefix = lightingTarget === "logo" ? "logo-"
                : lightingTarget === "ring" ? "ring-" : ""
            return `${prefix}segment-${lightingSegment}`
        }
        if (lightingTarget === "logo") return "logo-color"
        if (lightingTarget === "ring") return "ring-color"
        return "color"
    }

    function lightingEffectFeature() {
        if (lightingTarget === "logo") return "logo-effect"
        if (lightingTarget === "ring") return "ring-effect"
        return "effect"
    }

    function lightingSpeedFeature() {
        if (lightingTarget === "logo") return "logo-speed"
        if (lightingTarget === "ring") return "ring-speed"
        return "speed"
    }

    function selectedLightingEffect() {
        if (lightingTarget === "ring") return ringEffect
        if (lightingTarget === "logo") return logoEffect
        return logoEffect === ringEffect ? logoEffect : "mixed"
    }

    function selectedLightingSpeed() {
        if (lightingTarget === "ring") return ringSpeed
        if (lightingTarget === "logo") return logoSpeed
        return logoSpeed === ringSpeed ? logoSpeed : "mixed"
    }

    function zoneProfileKnown(zone) {
        const colors = zone === "ring" ? ringColors : logoColors
        const effect = zone === "ring" ? ringEffect : logoEffect
        const speed = zone === "ring" ? ringSpeed : logoSpeed
        return Array.isArray(colors) && colors.length === 5
            && typeof effect === "string" && typeof speed === "string"
    }

    function lightingTargetProfileKnown() {
        // Every non-recovery lighting command rebuilds and writes both zones,
        // so either unknown zone makes the entire cached profile unusable.
        return zoneProfileKnown("logo") && zoneProfileKnown("ring")
    }

    function canApplyLightingColor() {
        // `color` is the explicit synchronized recovery command and does not
        // read either cached zone. All other writes require known cache data.
        return lightingTarget === "both" || lightingTargetProfileKnown()
    }

    function normalizeColor(value) {
        const presets = {
            "blue": "#0029ff", "cyan": "#33ffcc", "magenta": "#ff00cc",
            "red": "#ff2020", "green": "#20ff66", "white": "#ffffff"
        }
        return presets[value] ?? (/^#[0-9a-fA-F]{6}$/.test(value) ? value : "#33ffcc")
    }

    function loadPicker() {
        // A new target or segment starts a new live-color sequence.
        lastLiveLightingKey = ""
        const selected = selectedLightingColor()
        if (typeof selected !== "string") return
        const hex = normalizeColor(selected)
        const red = parseInt(hex.slice(1, 3), 16) / 255
        const green = parseInt(hex.slice(3, 5), 16) / 255
        const blue = parseInt(hex.slice(5, 7), 16) / 255
        const maximum = Math.max(red, green, blue)
        const minimum = Math.min(red, green, blue)
        const delta = maximum - minimum
        let hue = 0
        if (delta > 0) {
            if (maximum === red) hue = ((green - blue) / delta) % 6
            else if (maximum === green) hue = (blue - red) / delta + 2
            else hue = (red - green) / delta + 4
            hue = ((hue / 6) + 1) % 1
        }
        pickerHue = hue
        pickerSaturation = maximum === 0 ? 0 : delta / maximum
        pickerValue = maximum
    }

    function colorComponent(value) {
        return Math.round(value * 255).toString(16).padStart(2, "0")
    }

    function pickerHex() {
        return `#${colorComponent(pickerColor.r)}${colorComponent(pickerColor.g)}${colorComponent(pickerColor.b)}`
    }

    function friendlyError(message, exitCode, context) {
        if (exitCode === 127)
            return "CLI soniccore não encontrada. Execute tools/install-user.sh."
        if (exitCode === 126)
            return "CLI soniccore sem permissão de execução. Execute tools/install-user.sh."
        const normalized = message.toLowerCase()
        if (normalized.includes("not found") || normalized.includes("não encontrado"))
            return "Dongle USB desconectado"
        if (normalized.includes("permission denied")
                || normalized.includes("permissão negada")
                || normalized.includes("operation not permitted"))
            return "Sem permissão para acessar o headset"
        if (context === "write")
            return "Não foi possível aplicar a configuração"
        return "Não foi possível ler o headset"
    }

    function normalizeBatteryPercent(value) {
        // `status --dry-run` and a cache recorded before a battery sample use
        // null. Check that sentinel before Number(), because Number(null) is
        // zero and would falsely show an empty battery as 0%.
        if (value === null || value === undefined
                || (typeof value === "string" && value.trim().length === 0)) return -1
        if (typeof value !== "number" && typeof value !== "string") {
            throw new Error("percentual inválido")
        }

        const percentage = Number(value)
        if (!Number.isFinite(percentage) || percentage < 0 || percentage > 100) {
            throw new Error("percentual inválido")
        }
        return percentage
    }

    function clearHeadsetState() {
        batteryPercent = -1
        charging = false
        headsetConnected = null
        rawFeature = ""
        ambientMode = "unknown"
        microphoneState = "unknown"
        sidetoneLevel = "unknown"
        lightingEnabled = null
        lightingColor = null
        logoColor = null
        ringColor = null
        logoColors = null
        ringColors = null
        logoEffect = null
        ringEffect = null
        logoSpeed = null
        ringSpeed = null
        gameChatValue = -1
        lightingSource = ""
        lightingUpdatedAtMs = 0
        lightingHeld = false
        powerSaving = ""
    }

    function applyResult(data) {
        const exitCode = Number(data["exit code"] ?? -1)
        const stdout = String(data.stdout ?? "").trim()
        const stderr = String(data.stderr ?? "").trim()
        if (exitCode !== 0) {
            clearHeadsetState()
            errorMessage = friendlyError(stderr, exitCode, "read")
            return
        }
        try {
            const result = JSON.parse(stdout)
            batteryPercent = normalizeBatteryPercent(result.battery_percent)
            if (batteryPercent === 0 && result.headset_connected !== true)
                batteryPercent = -1
            charging = result.charging === true
            rawFeature = String(result.raw_feature ?? "")
            ambientMode = String(result.ambient_mode ?? "unknown")
            headsetConnected = result.headset_connected ?? null
            lightingSource = String(result.lighting_source ?? "")
            lightingUpdatedAtMs = Number(result.lighting_updated_at_ms ?? 0)
            lightingHeld = result.lighting_held === true
            powerSaving = String(result.power_saving ?? "")
            nowMs = Date.now()
            microphoneState = String(result.microphone ?? "unknown")
            sidetoneLevel = String(result.sidetone_level ?? "unknown")
            lightingEnabled = result.lighting_enabled ?? null
            lightingColor = result.lighting_color ?? null
            logoColor = result.logo_color ?? null
            ringColor = result.ring_color ?? null
            logoColors = result.logo_colors ?? null
            ringColors = result.ring_colors ?? null
            logoEffect = result.logo_effect ?? null
            ringEffect = result.ring_effect ?? null
            logoSpeed = result.logo_speed ?? null
            ringSpeed = result.ring_speed ?? null
            gameChatValue = (typeof result.game_chat_value === "number" || (typeof result.game_chat_value === "string" && result.game_chat_value.trim() !== "")) && Number.isInteger(Number(result.game_chat_value)) && Number(result.game_chat_value) >= 0 && Number(result.game_chat_value) <= 16 ? Number(result.game_chat_value) : -1
            errorMessage = ""
        } catch (error) {
            clearHeadsetState()
            errorMessage = `Resposta inválida: ${error}`
        }
    }

    property Component compactRepresentation: MouseArea {
        id: compact

        implicitWidth: 34 + Kirigami.Units.smallSpacing
        implicitHeight: Math.max(compactLayout.implicitHeight, 18)
        Layout.minimumWidth: implicitWidth
        Layout.preferredWidth: implicitWidth
        Layout.minimumHeight: implicitHeight
        Layout.preferredHeight: implicitHeight
        onClicked: root.toggleRequested()

        Item {
            id: compactLayout
            anchors.centerIn: parent
            width: 34
            height: 18

            Item {
                anchors.fill: parent

                Rectangle {
                    id: batteryBody
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    width: 30
                    height: 16
                    radius: 3
                    color: "transparent"
                    border.width: 1
                    border.color: root.batteryColor

                    Rectangle {
                        x: 2
                        y: 2
                        width: root.batteryPercent >= 0
                            ? Math.max(2, (parent.width - 4) * root.batteryPercent / 100)
                            : 0
                        height: parent.height - 4
                        radius: 1
                        color: root.batteryColor
                        opacity: 0.85
                    }

                    Kirigami.Icon {
                        anchors.centerIn: parent
                        width: 11
                        height: 11
                        source: "audio-headphones"
                        visible: root.batteryPercent >= 0
                    }

                    PlasmaComponents.Label {
                        anchors.centerIn: parent
                        visible: root.batteryPercent < 0
                        text: "—"
                        color: root.batteryColor
                        font.bold: true
                    }

                    PlasmaComponents.Label {
                        visible: root.charging
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.rightMargin: 1
                        anchors.topMargin: -3
                        text: "⚡"
                        color: "#f5c542"
                        font.pixelSize: 8
                        font.bold: true
                    }
                }

                Rectangle {
                    anchors.left: batteryBody.right
                    anchors.leftMargin: 1
                    anchors.verticalCenter: parent.verticalCenter
                    width: 2
                    height: 7
                    radius: 1
                    color: root.batteryColor
                }
            }

        }
    }

    property Component fullRepresentation: Item {
        Layout.minimumWidth: Kirigami.Units.gridUnit * 21
        Layout.minimumHeight: expandedRepresentation.implicitHeight + Kirigami.Units.largeSpacing * 2
        Layout.preferredWidth: Kirigami.Units.gridUnit * 66
        Layout.preferredHeight: expandedRepresentation.implicitHeight + Kirigami.Units.largeSpacing * 2

        // Drawn behind every control; only with the Vivo theme.
        VivoBackground {
            anchors.fill: parent
            visible: root.vivo && root.vivoBackground !== "none"
            style: root.vivoBackground
            intensity: root.configuration.backgroundIntensity
            reactive: root.configuration.backgroundReactive
            running: root.vivo && root.expanded
            bass: vivoPanel.bass
            accentA: root.vivoAccentA
            accentB: root.vivoAccentB
            lightPalette: root.vivoPalette
            wallpaper: root.wallpaperUrl
        }

        GridLayout {
            id: expandedRepresentation
            // Fills the window: the columns stretch down to its bottom edge.
            anchors.fill: parent
            anchors.margins: Kirigami.Units.largeSpacing
            // Every section is shown at once: equalizer on the left, the headset
            // in the middle, the lights on the right. Narrower windows stack the
            // headset above the equalizer, then everything in one column.
            columns: width >= Kirigami.Units.gridUnit * 54 ? 3 : width >= Kirigami.Units.gridUnit * 38 ? 2 : 1
            columnSpacing: Kirigami.Units.largeSpacing
            rowSpacing: Kirigami.Units.largeSpacing
            readonly property real columnWidth: (width - columnSpacing * (columns - 1)) / columns
            readonly property int equalizerColumns: columnWidth >= Kirigami.Units.gridUnit * 16 ? 10 : 5

            ColumnLayout {
                Layout.row: 0
                Layout.column: expandedRepresentation.columns === 3 ? 1 : 0
                Layout.alignment: Qt.AlignTop
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: expandedRepresentation.columnWidth
                spacing: Kirigami.Units.largeSpacing

                // The headset overview, in a card like the other columns.
                SectionCard {
            PlasmaComponents.Label {
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumHeight: implicitHeight
                text: "JamBaLinux SonicCore"
                font.bold: true
                font.pixelSize: Kirigami.Units.gridUnit * 1.05
            }

            PlasmaComponents.Label {
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumHeight: implicitHeight
                text: root.outputIsQuantum ? "JBL Quantum 810 Wireless" : root.outputLabel
                elide: Text.ElideRight
                Layout.maximumWidth: parent.width
                opacity: 0.7
            }

            VivoPanel {
                id: vivoPanel
                visible: root.vivo
                Layout.fillWidth: true
                Layout.preferredHeight: implicitHeight
                running: root.vivo && root.expanded
                batteryPercent: root.batteryPercent
                charging: root.charging
                headsetConnected: root.headsetConnected
                batteryColor: root.batteryColor
                accentA: root.vivoAccentA
                accentB: root.vivoAccentB
                lightPalette: root.vivoPalette
                equipmentImage: root.equipmentImage
                pathCommand: root.soniccoreCommand(["audio-path", "--format", "json"])
                outputIsQuantum: root.outputIsQuantum
                outputIcon: root.outputIcon
                hoursLeftText: root.hoursLeftText
            }

            // The playing output when it is not the headset, with the icon
            // Plasma's volume applet uses for it.
            Kirigami.Icon {
                visible: !root.vivo && !root.outputIsQuantum
                source: root.outputIcon
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                Layout.preferredHeight: Kirigami.Units.gridUnit * 6
            }

            Image {
                visible: !root.vivo && root.outputIsQuantum
                source: root.equipmentImage
                // Same as the Vivo header: downscale the 1254 px photo once.
                sourceSize.width: Kirigami.Units.gridUnit * 16
                sourceSize.height: Kirigami.Units.gridUnit * 16
                fillMode: Image.PreserveAspectFit
                smooth: true
                mipmap: true
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: Kirigami.Units.gridUnit * 8
                Layout.preferredHeight: Kirigami.Units.gridUnit * 8
                Layout.maximumWidth: Kirigami.Units.gridUnit * 8
                Layout.maximumHeight: Kirigami.Units.gridUnit * 8
            }

            PlasmaComponents.Label {
                visible: !root.vivo
                Layout.fillWidth: true
                Layout.maximumHeight: implicitHeight
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                // Off, the last reading goes stale while it charges.
                text: root.headsetConnected === false
                    ? (root.charging ? "Desligado · carregando" : "Desligado")
                    : root.batteryPercent >= 0 ? `${root.batteryPercent}%` : "—"
                color: root.batteryColor
                font.pixelSize: Kirigami.Units.gridUnit * 1.7
                font.bold: true
            }

            PlasmaComponents.Label {
                visible: !root.vivo && !root.outputIsQuantum
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumHeight: implicitHeight
                text: "bateria do JBL Quantum 810"
                opacity: 0.6
            }

            PlasmaComponents.Label {
                visible: !root.vivo && root.hoursLeftText.length > 0
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumHeight: implicitHeight
                text: root.hoursLeftText
                opacity: 0.7
            }

            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumHeight: implicitHeight
                spacing: Kirigami.Units.largeSpacing

                PlasmaComponents.Label {
                    text: root.headsetConnected === null
                        ? "Headset: aguardando"
                        : root.headsetConnected ? "● Ligado" : "○ Desligado"
                    color: root.headsetConnected === true
                        ? Kirigami.Theme.positiveTextColor
                        : Kirigami.Theme.disabledTextColor
                    font.bold: root.headsetConnected === true
                }

                PlasmaComponents.Label {
                    text: root.microphoneState === "active"
                        ? "● Microfone ativo"
                        : root.microphoneState === "muted" ? "● Microfone mudo" : "Microfone aguardando"
                    color: root.microphoneState === "muted"
                        ? Kirigami.Theme.negativeTextColor
                        : root.microphoneState === "active"
                            ? Kirigami.Theme.positiveTextColor
                            : Kirigami.Theme.disabledTextColor
                    font.bold: root.microphoneState !== "unknown"
                }
            }

            PlasmaComponents.Label {
                Layout.fillWidth: true
                Layout.minimumHeight: Kirigami.Units.gridUnit
                Layout.preferredHeight: Kirigami.Units.gridUnit
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                text: root.actionMessage
                opacity: root.actionMessage.length > 0 ? 1 : 0
                color: root.actionFailed
                    ? Kirigami.Theme.negativeTextColor
                    : Kirigami.Theme.positiveTextColor
                wrapMode: Text.Wrap
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.maximumHeight: implicitHeight
                spacing: Kirigami.Units.smallSpacing

                RowLayout {
                    Layout.fillWidth: true

                    PlasmaComponents.Label {
                        text: "Chat"
                        font.bold: root.gameChatValue >= 0 && root.gameChatValue < 8
                    }

                    Item { Layout.fillWidth: true }

                    PlasmaComponents.Label {
                        text: root.gameChatValue < 0
                            ? "Mova o dial para detectar"
                            : root.gameChatValue === 8 ? "Centro" : `${root.gameChatValue}/16`
                        opacity: 0.7
                    }

                    Item { Layout.fillWidth: true }

                    PlasmaComponents.Label {
                        text: "Game"
                        font.bold: root.gameChatValue > 8
                    }
                }

                Item {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 16

                    Rectangle {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        height: 6
                        radius: 3
                        color: Kirigami.Theme.disabledTextColor
                        opacity: 0.35
                    }

                    Rectangle {
                        visible: root.gameChatValue >= 0
                        x: (parent.width - width) * Math.max(0, Math.min(16, root.gameChatValue)) / 16
                        anchors.verticalCenter: parent.verticalCenter
                        width: 14
                        height: 14
                        radius: 7
                        color: Kirigami.Theme.highlightColor
                        border.width: 2
                        border.color: Kirigami.Theme.backgroundColor
                    }
                }
            }



            PlasmaComponents.Label {
                Layout.alignment: Qt.AlignHCenter
                visible: root.charging
                text: root.batteryPercent >= 100 ? "⚡ Carregado" : "⚡ Carregando"
                color: "#f5c542"
                font.bold: true
            }

            PlasmaComponents.Label {
                Layout.fillWidth: true
                visible: root.errorMessage.length > 0 || !root.daemonAvailable
                horizontalAlignment: Text.AlignHCenter
                text: root.errorMessage.length > 0
                    ? root.errorMessage
                    : "Monitor em tempo real inativo"
                color: root.errorMessage.length > 0
                    ? Kirigami.Theme.negativeTextColor
                    : Kirigami.Theme.neutralTextColor
                wrapMode: Text.Wrap
                opacity: 0.85
            }
                }

                SectionCard {
                    title: "Som ambiente"
                    subtitle: `Atual: ${root.ambientLabel(root.ambientMode)}`
                    enabled: root.deviceAvailable

                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                        enabled: root.deviceAvailable && !root.controlBusy
                        PlasmaComponents.Button { text: "Desligado"; checked: root.ambientMode === "off"; onClicked: root.runControl("ambient", "off") }
                        PlasmaComponents.Button { text: "ANC"; checked: root.ambientMode === "anc"; onClicked: root.runControl("ambient", "anc") }
                        PlasmaComponents.Button { text: "TalkThru"; checked: root.ambientMode === "talkthru"; onClicked: root.runControl("ambient", "talkthru") }
                    }
                }

                SectionCard {
                    // The last card of each column stretches to the bottom,
                    // so the three columns end level.
                    Layout.fillHeight: true
                    title: "Retorno do microfone"
                    subtitle: `Atual: ${root.sidetoneLabel(root.sidetoneLevel)}`
                    enabled: root.deviceAvailable

                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                        enabled: root.deviceAvailable && !root.controlBusy
                        PlasmaComponents.Button { text: "Off"; checked: root.sidetoneLevel === "off"; onClicked: root.runControl("sidetone", "off") }
                        PlasmaComponents.Button { text: "Baixo"; checked: root.sidetoneLevel === "low"; onClicked: root.runControl("sidetone", "low") }
                        PlasmaComponents.Button { text: "Médio"; checked: root.sidetoneLevel === "medium"; onClicked: root.runControl("sidetone", "medium") }
                        PlasmaComponents.Button { text: "Alto"; checked: root.sidetoneLevel === "high"; onClicked: root.runControl("sidetone", "high") }
                    }
                }
            }

            ColumnLayout {
                Layout.row: expandedRepresentation.columns === 3 ? 0 : 1
                Layout.column: 0
                Layout.alignment: Qt.AlignTop
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: expandedRepresentation.columnWidth
                spacing: Kirigami.Units.largeSpacing

                SectionCard {
                    title: "Equalizador de saída"
                    subtitle: root.equalizerBypassed
                        ? "Desligado: som natural do sistema, sem equalização. Seus ajustes ficam guardados."
                        : ""

                    PlasmaComponents.Switch {
                        id: equalizerToggle
                        Layout.alignment: Qt.AlignHCenter
                        text: "Equalizador ligado"
                        checked: !root.equalizerBypassed
                        enabled: !root.equalizerBusy && !root.equalizerUpdating
                        onClicked: {
                            const on = checked
                            checked = Qt.binding(function() { return !root.equalizerBypassed })
                            root.setEqualizerBypassed(!on)
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        id: equalizerBody
                        // Off, the profiles and bands fold away (the card animates).
                        visible: !root.equalizerBypassed
                        // Narrow card: the profile actions show only their icons
                        // (name in the tooltip) instead of pushing the card wider.
                        readonly property bool compact: width < Kirigami.Units.gridUnit * 22
                        // Only profile actions below are blocked while one runs: the band sliders
                        // keep moving and queue their latest value (pendingEqualizerBands).
                        spacing: Kirigami.Units.smallSpacing

                        PlasmaComponents.ComboBox {
                            id: equalizerProfileSelector
                            enabled: !root.equalizerBusy && !root.equalizerUpdating
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                            model: equalizerProfileModel
                            textRole: "label"
                            valueRole: "profileId"
                            // Taken from the canonical selection rather than from
                            // currentText, which follows an index the control may
                            // have rewritten while the model was being rebuilt.
                            displayText: root.equalizerSelectionLabel()

                            // Reasserted after every catalog rebuild and after every
                            // confirmed selection change, for the same reason.
                            function synchronizeSelection() {
                                currentIndex = root.equalizerProfileIndex(
                                    root.equalizerActiveProfileId)
                            }

                            Component.onCompleted: equalizerProfileSelector.synchronizeSelection()
                            onCountChanged: equalizerProfileSelector.synchronizeSelection()
                            onActivated: function(index) {
                                const profile = equalizerProfileModel.get(index)
                                if (profile === null || profile === undefined) return
                                root.applyEqualizerProfile(profile.profileId, profile.factory)
                                // The selector never leads the CLI: the confirmed
                                // entry stays shown until a refresh reports the new
                                // selection, including when the request is refused.
                                equalizerProfileSelector.synchronizeSelection()
                            }

                            Connections {
                                target: root

                                function onEqualizerProfileRevisionChanged() {
                                    equalizerProfileSelector.synchronizeSelection()
                                }

                                function onEqualizerActiveProfileIdChanged() {
                                    equalizerProfileSelector.synchronizeSelection()
                                }
                            }
                        }

                        RowLayout {
                            enabled: !root.equalizerBusy && !root.equalizerUpdating
                            Layout.alignment: Qt.AlignHCenter
                            Layout.maximumWidth: equalizerBody.width

                            PlasmaComponents.Button {
                                text: "Criar perfil"
                                display: equalizerBody.compact ? QQC2.AbstractButton.IconOnly : QQC2.AbstractButton.TextBesideIcon
                                QQC2.ToolTip.text: text
                                QQC2.ToolTip.visible: equalizerBody.compact && hovered
                                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                                icon.name: "list-add"
                                checkable: true
                                checked: root.equalizerCreateVisible
                                onClicked: {
                                    const opening = !root.equalizerCreateVisible
                                    root.closeEqualizerProfileEditors()
                                    root.equalizerCreateVisible = opening
                                }
                            }

                            PlasmaComponents.Button {
                                // Factory presets are immutable: the library
                                // actions exist only for user-owned profiles.
                                visible: root.selectedEqualizerProfileIsCustom()
                                text: "Atualizar"
                                display: equalizerBody.compact ? QQC2.AbstractButton.IconOnly : QQC2.AbstractButton.TextBesideIcon
                                QQC2.ToolTip.text: text
                                QQC2.ToolTip.visible: equalizerBody.compact && hovered
                                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                                icon.name: "document-save"
                                onClicked: root.updateEqualizerProfile()
                            }

                            PlasmaComponents.Button {
                                visible: root.selectedEqualizerProfileIsCustom()
                                text: "Renomear"
                                display: equalizerBody.compact ? QQC2.AbstractButton.IconOnly : QQC2.AbstractButton.TextBesideIcon
                                QQC2.ToolTip.text: text
                                QQC2.ToolTip.visible: equalizerBody.compact && hovered
                                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                                icon.name: "edit-rename"
                                checkable: true
                                checked: root.equalizerRenameVisible
                                onClicked: {
                                    const opening = !root.equalizerRenameVisible
                                    const profile = root.selectedEqualizerProfile()
                                    root.closeEqualizerProfileEditors()
                                    root.equalizerRenameVisible = opening
                                    if (opening && profile !== null)
                                        root.equalizerProfileNameDraft = String(profile.label)
                                }
                            }

                            PlasmaComponents.Button {
                                visible: root.selectedEqualizerProfileIsCustom()
                                text: root.equalizerDeleteConfirmation
                                    ? "Confirmar exclusão" : "Excluir"
                                icon.name: "edit-delete"
                                // The confirmation always says what it does.
                                display: equalizerBody.compact && !root.equalizerDeleteConfirmation
                                    ? QQC2.AbstractButton.IconOnly : QQC2.AbstractButton.TextBesideIcon
                                QQC2.ToolTip.text: text
                                QQC2.ToolTip.visible: equalizerBody.compact && hovered
                                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                                onClicked: {
                                    if (root.equalizerDeleteConfirmation) {
                                        root.equalizerDeleteConfirmation = false
                                        root.deleteEqualizerProfile()
                                    } else {
                                        root.closeEqualizerProfileEditors()
                                        root.equalizerDeleteConfirmation = true
                                    }
                                }
                            }
                        }

                        RowLayout {
                            enabled: !root.equalizerBusy && !root.equalizerUpdating
                            Layout.fillWidth: true
                            visible: root.equalizerCreateVisible || root.equalizerRenameVisible
                            onVisibleChanged: {
                                if (!visible) return
                                // Opening the editor always shows the draft that the
                                // action prepared, never the text left behind by an
                                // earlier edit.
                                equalizerProfileNameField.adoptDraft()
                                equalizerProfileNameField.forceActiveFocus()
                            }

                            PlasmaComponents.TextField {
                                id: equalizerProfileNameField
                                Layout.fillWidth: true
                                placeholderText: root.equalizerCreateVisible
                                    ? "Nome do novo perfil" : "Nome do perfil"
                                // The CLI rejects longer names; stopping the entry
                                // here avoids a rejected round trip.
                                maximumLength: 64

                                // The draft on root is the single source of truth.
                                // Interactive editing writes `text` directly, which
                                // would drop a declarative binding on it and leave
                                // the field showing a stale name the next time an
                                // editor is opened, so both directions are wired
                                // explicitly. The equality guard keeps the two
                                // assignments from looping and leaves the cursor
                                // alone while the user is typing.
                                function adoptDraft() {
                                    if (text !== root.equalizerProfileNameDraft)
                                        text = root.equalizerProfileNameDraft
                                }

                                Component.onCompleted: equalizerProfileNameField.adoptDraft()
                                onTextChanged: root.equalizerProfileNameDraft = text
                                onAccepted: root.submitEqualizerProfileName()

                                Connections {
                                    target: root

                                    function onEqualizerProfileNameDraftChanged() {
                                        equalizerProfileNameField.adoptDraft()
                                    }
                                }
                            }

                            PlasmaComponents.Button {
                                text: root.equalizerCreateVisible ? "Criar" : "Salvar"
                                enabled: root.equalizerProfileNameDraft.trim().length > 0
                                onClicked: root.submitEqualizerProfileName()
                            }

                            PlasmaComponents.Button {
                                text: "Cancelar"
                                icon.name: "dialog-cancel"
                                onClicked: root.closeEqualizerProfileEditors()
                            }
                        }

                        PlasmaComponents.Label {
                            Layout.fillWidth: true
                            visible: root.equalizerDeleteConfirmation
                                && root.selectedEqualizerProfileIsCustom()
                            text: `Excluir o perfil “${root.equalizerSelectionLabel()}”? Clique em “Confirmar exclusão” para remover definitivamente.`
                            wrapMode: Text.Wrap
                            horizontalAlignment: Text.AlignHCenter
                            color: Kirigami.Theme.negativeTextColor
                        }

                        GridLayout {
                            Layout.fillWidth: true
                            Layout.preferredHeight: Kirigami.Units.gridUnit
                                * (expandedRepresentation.equalizerColumns === 10 ? 14 : 29)
                            columns: expandedRepresentation.equalizerColumns
                            columnSpacing: Kirigami.Units.smallSpacing
                            rowSpacing: Kirigami.Units.largeSpacing

                            Repeater {
                                model: equalizerBandModel

                                delegate: ColumnLayout {
                                    id: equalizerBand
                                    required property int frequencyHz
                                    required property real gainDb
                                    Layout.fillWidth: true
                                    Layout.minimumWidth: Kirigami.Units.gridUnit * 1.25
                                    Layout.preferredHeight: Kirigami.Units.gridUnit * 14
                                    spacing: Kirigami.Units.smallSpacing

                                    function synchronizeHandle() {
                                        if (!equalizerSlider.pressed)
                                            equalizerSlider.value = Number(gainDb)
                                    }

                                    Component.onCompleted: synchronizeHandle()
                                    onGainDbChanged: synchronizeHandle()

                                    Connections {
                                        target: root

                                        function onEqualizerVisualRevisionChanged() {
                                            equalizerBand.synchronizeHandle()
                                        }
                                    }

                                    Timer {
                                        id: equalizerCommit
                                        // Like a radio's volume knob: while the handle is
                                        // dragged the band is sent at most every 70 ms, not
                                        // only on release. The queue keeps just the latest
                                        // value per band, so commands never pile up.
                                        interval: 70
                                        repeat: false
                                        onTriggered: root.setEqualizerBand(
                                            equalizerBand.frequencyHz,
                                            equalizerSlider.value)
                                    }

                                    PlasmaComponents.Label {
                                        Layout.fillWidth: true
                                        horizontalAlignment: Text.AlignHCenter
                                        text: `${Number(equalizerSlider.value) >= 0 ? "+" : ""}${Number(equalizerSlider.value).toFixed(0)}`
                                        font.family: "monospace"
                                        font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                                    }

                                    PlasmaComponents.Slider {
                                        id: equalizerSlider
                                        Layout.alignment: Qt.AlignHCenter
                                        Layout.minimumHeight: Kirigami.Units.gridUnit * 8
                                        Layout.preferredHeight: Kirigami.Units.gridUnit * 10
                                        orientation: Qt.Vertical
                                        from: -12
                                        to: 12
                                        stepSize: 1
                                        value: 0
                                        // Throttle, not debounce: a running timer is left alone so
                                        // a continuous drag still fires every interval.
                                        onMoved: {
                                            if (!equalizerCommit.running) equalizerCommit.start()
                                        }
                                        onPressedChanged: {
                                            // Release sends the final position right away.
                                            if (!pressed) {
                                                equalizerCommit.stop()
                                                root.setEqualizerBand(equalizerBand.frequencyHz, value)
                                            }
                                        }
                                    }

                                    PlasmaComponents.Label {
                                        Layout.fillWidth: true
                                        horizontalAlignment: Text.AlignHCenter
                                        text: root.equalizerCompactFrequencyLabel(
                                            equalizerBand.frequencyHz)
                                        font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                                    }
                                }
                            }
                        }

                        PlasmaComponents.Button {
                            enabled: !root.equalizerBusy && !root.equalizerUpdating
                            Layout.alignment: Qt.AlignHCenter
                            text: "Zerar bandas"
                            icon.name: "edit-clear"
                            onClicked: root.resetEqualizer()
                        }

                        PlasmaComponents.Label {
                            Layout.fillWidth: true
                            visible: root.equalizerProblemText().length > 0
                            text: root.equalizerProblemText()
                            wrapMode: Text.Wrap
                            horizontalAlignment: Text.AlignHCenter
                            color: Kirigami.Theme.negativeTextColor
                        }
                    }
                }

                SectionCard {
                    // The last card of each column stretches to the bottom,
                    // so the three columns end level.
                    Layout.fillHeight: true
                    title: "Áudio espacial"
                    subtitle: root.spatialStatusText()

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Kirigami.Units.smallSpacing

                        // Listening modes: Natural switches the 3D off. Narrow
                        // cards wrap them into two rows instead of cutting one.
                        GridLayout {
                            id: listeningModes
                            Layout.alignment: Qt.AlignHCenter
                            Layout.maximumWidth: parent.width
                            enabled: !root.spatialBusy && !root.spatialUpdating
                                && (root.spatialEnabled || root.spatialReady)
                            columns: parent.width >= Kirigami.Units.gridUnit * 22 ? 6 : 3
                            columnSpacing: Kirigami.Units.smallSpacing
                            rowSpacing: Kirigami.Units.smallSpacing
                            uniformCellWidths: columns === 3

                            Repeater {
                                model: [
                                    { "mode": "natural", "label": "Natural" },
                                    { "mode": "game", "label": "Jogo" },
                                    { "mode": "cinema", "label": "Cinema" },
                                    { "mode": "music", "label": "Música" },
                                    { "mode": "custom", "label": "Personalizado" }
                                ]

                                PlasmaComponents.Button {
                                    required property var modelData
                                    Layout.fillWidth: listeningModes.columns === 3
                                    text: modelData.label
                                    checked: modelData.mode === "natural"
                                        ? !root.spatialEnabled
                                        : root.spatialEnabled && root.spatialListening === modelData.mode
                                    onClicked: root.selectListeningMode(modelData.mode)
                                }
                            }

                            PlasmaComponents.BusyIndicator {
                                running: root.spatialBusy || root.spatialUpdating
                                visible: running
                                Layout.preferredWidth: Kirigami.Units.iconSizes.small
                                Layout.preferredHeight: Kirigami.Units.iconSizes.small
                            }
                        }

                        // Fine controls, shown while the 3D is on. Moving one
                        // turns the mode into "Personalizado".
                        GridLayout {
                            Layout.fillWidth: true
                            visible: root.spatialEnabled
                            enabled: !root.spatialBusy
                            columns: 3
                            columnSpacing: Kirigami.Units.smallSpacing
                            rowSpacing: 0

                            Repeater {
                                model: [
                                    { "control": "intensity", "key": "intensity", "label": "Intensidade 3D", "from": 0.1, "to": 1, "step": 0.05, "percent": true },
                                    { "control": "dialogue", "key": "dialogue_db", "label": "Diálogos", "from": -12, "to": 6, "step": 0.5, "percent": false },
                                    { "control": "bass", "key": "bass_db", "label": "Graves", "from": -12, "to": 6, "step": 0.5, "percent": false },
                                    { "control": "surround", "key": "surround_db", "label": "Surround", "from": -12, "to": 6, "step": 0.5, "percent": false }
                                ]

                                delegate: Item {
                                    id: soundRow
                                    required property var modelData
                                    Layout.columnSpan: 3
                                    Layout.fillWidth: true
                                    implicitHeight: soundRowLayout.implicitHeight

                                    RowLayout {
                                        id: soundRowLayout
                                        anchors.left: parent.left
                                        anchors.right: parent.right

                                        PlasmaComponents.Label {
                                            text: soundRow.modelData.label
                                            Layout.preferredWidth: Kirigami.Units.gridUnit * 5.5
                                            elide: Text.ElideRight
                                        }

                                        PlasmaComponents.Slider {
                                            id: soundSlider
                                            Layout.fillWidth: true
                                            // Takes the room that is left; never pushes
                                            // the value off the card.
                                            Layout.preferredWidth: Kirigami.Units.gridUnit * 4
                                            Layout.minimumWidth: Kirigami.Units.gridUnit * 3
                                            from: soundRow.modelData.from
                                            to: soundRow.modelData.to
                                            stepSize: soundRow.modelData.step
                                            value: Number(root.spatialSound[soundRow.modelData.key] ?? 0)
                                            // Sent once, when the handle is let go.
                                            onPressedChanged: {
                                                if (pressed) return
                                                root.setSpatialSoundControl(soundRow.modelData.control, value)
                                                // Follow the confirmed value again after the drag.
                                                value = Qt.binding(() => Number(root.spatialSound[soundRow.modelData.key] ?? 0))
                                            }
                                        }

                                        PlasmaComponents.Label {
                                            Layout.preferredWidth: Kirigami.Units.gridUnit * 3.5
                                            horizontalAlignment: Text.AlignRight
                                            text: soundRow.modelData.percent
                                                ? `${Math.round(soundSlider.value * 100)}%`
                                                : `${soundSlider.value > 0 ? "+" : ""}${soundSlider.value.toFixed(1)} dB`
                                            opacity: 0.7
                                        }
                                    }
                                }
                            }
                        }

                        PlasmaComponents.Label {
                            Layout.fillWidth: true
                            visible: !root.spatialActive && root.spatialError.length > 0
                            text: root.spatialError
                            wrapMode: Text.Wrap
                            horizontalAlignment: Text.AlignHCenter
                            color: Kirigami.Theme.negativeTextColor
                        }

                        PlasmaComponents.Label {
                            Layout.fillWidth: true
                            text: "Nos jogos, escolha a saída espacial JamBaLinux."
                            wrapMode: Text.Wrap
                            horizontalAlignment: Text.AlignHCenter
                            opacity: 0.6
                        }
                    }
                }
            }

            ColumnLayout {
                Layout.row: expandedRepresentation.columns === 1 ? 2 : 0
                Layout.column: expandedRepresentation.columns - 1
                Layout.alignment: Qt.AlignTop
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: expandedRepresentation.columnWidth
                spacing: Kirigami.Units.largeSpacing

                SectionCard {
                    // The last card of each column stretches to the bottom,
                    // so the three columns end level.
                    Layout.fillHeight: true
                    title: "Iluminação"
                    subtitle: root.lightingEnabled === null ? "Aguardando estado" : root.lightingEnabled ? "Atual: ligada" : "Atual: desligada"
                    enabled: root.deviceAvailable

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        visible: root.openRgbControlsLighting
                        text: "O OpenRGB está controlando as luzes agora. Mudanças feitas aqui seriam sobrescritas pelo próximo quadro dele."
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        color: Kirigami.Theme.neutralTextColor
                    }

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        visible: root.powerSaving.length > 0
                        text: root.powerSaving === "low-battery"
                            ? "Luzes apagadas para economizar bateria (bateria baixa). Elas voltam ao carregar; ligar agora desativa a economia até lá."
                            : "Luzes apagadas para economizar bateria (sem som). Elas voltam quando o som voltar; ligar agora mantém acesas."
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        opacity: 0.8
                    }

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        visible: root.lightingHeld
                        text: "Você assumiu as luzes; o OpenRGB está pausado."
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        opacity: 0.8
                    }

                    PlasmaComponents.Button {
                        Layout.alignment: Qt.AlignHCenter
                            && (root.openRgbControlsLighting || root.lightingHeld)
                        enabled: !root.controlBusy
                        text: root.lightingHeld ? "Devolver ao OpenRGB" : "Assumir controle"
                        icon.name: root.lightingHeld ? "edit-undo" : "object-locked"
                        onClicked: root.runLightingHold(!root.lightingHeld)
                    }

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        visible: text.length > 0
                        text: root.openRgbSummary()
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        opacity: 0.8
                    }

                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                            && (root.openRgbLinking || root.openRgbCanLink())

                        PlasmaComponents.BusyIndicator {
                            visible: root.openRgbLinking
                            running: visible
                            Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                            Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                        }

                        PlasmaComponents.Button {
                            enabled: !root.openRgbLinking
                            text: root.openRgbStatus?.plugin_installed === true
                                ? "Atualizar o plugin do OpenRGB" : "Vincular com OpenRGB"
                            icon.name: "insert-link"
                            onClicked: root.linkOpenRgb()
                        }
                    }

                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                        enabled: root.lightingWritable && !root.controlBusy
                        PlasmaComponents.Button { text: "Ligar"; icon.source: Qt.resolvedUrl("../images/light-on.svg"); checked: root.lightingEnabled === true; onClicked: root.runControl("lighting", "on") }
                        PlasmaComponents.Button { text: "Desligar"; icon.source: Qt.resolvedUrl("../images/light-off.svg"); checked: root.lightingEnabled === false; onClicked: root.runControl("lighting", "off") }
                    }

                    PlasmaComponents.Label {
                        visible: root.lightingEnabled !== false
                        Layout.alignment: Qt.AlignHCenter
                        text: "Cor sólida personalizada"
                        opacity: 0.7
                    }

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        visible: root.lightingEnabled !== false && !root.lightingTargetProfileKnown()
                        text: "Perfil da zona desconhecido. Aplique uma cor sólida em Ambos para recuperar."
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        color: Kirigami.Theme.negativeTextColor
                    }

                    RowLayout {
                        visible: root.lightingEnabled !== false
                        Layout.alignment: Qt.AlignHCenter
                        enabled: root.lightingWritable && !root.controlBusy

                        PlasmaComponents.Button {
                            text: "Ambos"
                            checkable: true
                            checked: root.lightingTarget === "both"
                            onClicked: {
                                root.lightingTarget = "both"
                                root.lightingSegment = -1
                                root.loadPicker()
                            }
                        }
                        PlasmaComponents.Button {
                            text: "Logotipo"
                            checkable: true
                            checked: root.lightingTarget === "logo"
                            onClicked: {
                                root.lightingTarget = "logo"
                                root.lightingSegment = -1
                                root.loadPicker()
                            }
                        }
                        PlasmaComponents.Button {
                            text: "Anel / fundo"
                            checkable: true
                            checked: root.lightingTarget === "ring"
                            onClicked: {
                                root.lightingTarget = "ring"
                                root.lightingSegment = -1
                                root.loadPicker()
                            }
                        }
                    }

                    PlasmaComponents.Label {
                        Layout.alignment: Qt.AlignHCenter
                        visible: root.lightingEnabled !== false && root.lightingTargetProfileKnown()
                        text: "Efeito"
                        opacity: 0.7
                    }

                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                        visible: root.lightingEnabled !== false && root.lightingTargetProfileKnown()
                        enabled: root.lightingWritable && !root.controlBusy && root.lightingTargetProfileKnown()
                        spacing: Kirigami.Units.smallSpacing

                        PlasmaComponents.Button { text: "Respiração"; checked: root.selectedLightingEffect() === "breathing"; onClicked: root.runControl(root.lightingEffectFeature(), "breathing") }
                        PlasmaComponents.Button { text: "Sólido"; checked: root.selectedLightingEffect() === "solid"; onClicked: root.runControl(root.lightingEffectFeature(), "solid") }
                        PlasmaComponents.Button { text: "Onda"; checked: root.selectedLightingEffect() === "wave"; onClicked: root.runControl(root.lightingEffectFeature(), "wave") }
                        PlasmaComponents.Button { text: "Falha"; checked: root.selectedLightingEffect() === "glitch"; onClicked: root.runControl(root.lightingEffectFeature(), "glitch") }
                    }

                    PlasmaComponents.Label {
                        Layout.alignment: Qt.AlignHCenter
                        visible: root.lightingEnabled !== false && root.lightingTargetProfileKnown()
                        text: "Velocidade"
                        opacity: 0.7
                    }

                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                        visible: root.lightingEnabled !== false && root.lightingTargetProfileKnown()
                        enabled: root.lightingWritable && !root.controlBusy
                        spacing: Kirigami.Units.smallSpacing

                        PlasmaComponents.Button { text: "0,5×"; checked: root.selectedLightingSpeed() === "0.5"; onClicked: root.runControl(root.lightingSpeedFeature(), "0.5") }
                        PlasmaComponents.Button { text: "1×"; checked: root.selectedLightingSpeed() === "1.0"; onClicked: root.runControl(root.lightingSpeedFeature(), "1.0") }
                        PlasmaComponents.Button { text: "1,5×"; checked: root.selectedLightingSpeed() === "1.5"; onClicked: root.runControl(root.lightingSpeedFeature(), "1.5") }
                        PlasmaComponents.Button { text: "2×"; checked: root.selectedLightingSpeed() === "2.0"; onClicked: root.runControl(root.lightingSpeedFeature(), "2.0") }
                    }

                    PlasmaComponents.Label {
                        visible: root.lightingEnabled !== false
                        Layout.alignment: Qt.AlignHCenter
                        text: root.lightingSegment < 0
                            ? "Cor: todos os segmentos"
                            : `Cor: segmento ${root.lightingSegment + 1}`
                        opacity: 0.7
                    }

                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                        visible: root.lightingEnabled !== false && root.lightingTargetProfileKnown()
                        enabled: root.lightingWritable && !root.controlBusy
                        spacing: Kirigami.Units.smallSpacing

                        PlasmaComponents.Button {
                            text: "Todos"
                            checkable: true
                            checked: root.lightingSegment < 0
                            onClicked: {
                                root.lightingSegment = -1
                                root.loadPicker()
                            }
                        }

                        Repeater {
                            model: 5

                            delegate: Rectangle {
                                required property int index
                                readonly property var colors: root.lightingTarget === "ring"
                                    ? root.ringColors : root.logoColors
                                Layout.preferredWidth: 34
                                Layout.preferredHeight: 30
                                radius: Kirigami.Units.cornerRadius
                                color: Array.isArray(colors) ? colors[index] ?? "transparent" : "transparent"
                                border.width: root.lightingSegment === index ? 3 : 1
                                border.color: root.lightingSegment === index
                                    ? Kirigami.Theme.highlightColor : Kirigami.Theme.textColor

                                PlasmaComponents.Label {
                                    anchors.centerIn: parent
                                    text: parent.index + 1
                                    color: "white"
                                    style: Text.Outline
                                    styleColor: "black"
                                    font.bold: true
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: {
                                        root.lightingSegment = parent.index
                                        root.loadPicker()
                                    }
                                }
                            }
                        }
                    }

                    Item {
                        visible: root.lightingEnabled !== false
                        id: colorPicker
                        Layout.alignment: Qt.AlignHCenter
                        Layout.preferredWidth: 220
                        Layout.preferredHeight: 220
                        // Stays usable while a write runs: the latest color is
                        // queued and sent as soon as the headset is free.
                        enabled: root.lightingWritable && root.canApplyLightingColor()

                        readonly property real centerX: width / 2
                        readonly property real centerY: height / 2
                        readonly property real wheelRadius: 91
                        readonly property real ringWidth: 25
                        readonly property real squareSize: 106

                        Canvas {
                            id: hueCanvas
                            anchors.fill: parent

                            onPaint: {
                                const ctx = getContext("2d")
                                ctx.clearRect(0, 0, width, height)
                                ctx.lineWidth = colorPicker.ringWidth
                                for (let degree = 0; degree < 360; ++degree) {
                                    const start = (degree - 1) * Math.PI / 180
                                    const end = (degree + 1) * Math.PI / 180
                                    ctx.beginPath()
                                    ctx.strokeStyle = Qt.hsla(degree / 360, 1, 0.5, 1)
                                    ctx.arc(colorPicker.centerX, colorPicker.centerY,
                                        colorPicker.wheelRadius, start, end)
                                    ctx.stroke()
                                }
                            }
                        }

                        Canvas {
                            id: svCanvas
                            width: colorPicker.squareSize
                            height: colorPicker.squareSize
                            anchors.centerIn: parent

                            onPaint: {
                                const ctx = getContext("2d")
                                ctx.clearRect(0, 0, width, height)
                                ctx.fillStyle = Qt.hsva(root.pickerHue, 1, 1, 1)
                                ctx.fillRect(0, 0, width, height)

                                const saturation = ctx.createLinearGradient(0, 0, width, 0)
                                saturation.addColorStop(0, "white")
                                saturation.addColorStop(1, "transparent")
                                ctx.fillStyle = saturation
                                ctx.fillRect(0, 0, width, height)

                                const value = ctx.createLinearGradient(0, 0, 0, height)
                                value.addColorStop(0, "transparent")
                                value.addColorStop(1, "black")
                                ctx.fillStyle = value
                                ctx.fillRect(0, 0, width, height)
                            }
                        }

                        Connections {
                            target: root
                            function onPickerHueChanged() { svCanvas.requestPaint() }
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.CrossCursor

                            function selectAt(pointerX, pointerY) {
                                const offsetX = pointerX - colorPicker.centerX
                                const offsetY = pointerY - colorPicker.centerY
                                const distance = Math.sqrt(offsetX * offsetX + offsetY * offsetY)
                                const innerRadius = colorPicker.wheelRadius - colorPicker.ringWidth / 2
                                const outerRadius = colorPicker.wheelRadius + colorPicker.ringWidth / 2
                                if (distance >= innerRadius && distance <= outerRadius) {
                                    root.pickerHue = (Math.atan2(offsetY, offsetX) / (2 * Math.PI) + 1) % 1
                                    // From black or gray every hue is still black or gray:
                                    // picking a hue on the ring means "this color", so make
                                    // it visible instead of silently writing black again.
                                    if (root.pickerValue < 0.05) root.pickerValue = 1
                                    if (root.pickerSaturation < 0.05) root.pickerSaturation = 1
                                    svCanvas.requestPaint()
                                    root.queueLightingColor()
                                    return
                                }

                                const left = colorPicker.centerX - colorPicker.squareSize / 2
                                const top = colorPicker.centerY - colorPicker.squareSize / 2
                                if (pointerX >= left && pointerX <= left + colorPicker.squareSize
                                        && pointerY >= top && pointerY <= top + colorPicker.squareSize) {
                                    root.pickerSaturation = Math.max(0, Math.min(1,
                                        (pointerX - left) / colorPicker.squareSize))
                                    root.pickerValue = Math.max(0, Math.min(1,
                                        1 - (pointerY - top) / colorPicker.squareSize))
                                    root.queueLightingColor()
                                }
                            }

                            onPressed: function(mouse) { selectAt(mouse.x, mouse.y) }
                            onPositionChanged: function(mouse) {
                                if (pressed) selectAt(mouse.x, mouse.y)
                            }
                        }

                        Rectangle {
                            width: 13
                            height: 13
                            radius: width / 2
                            x: colorPicker.centerX
                                + Math.cos(root.pickerHue * 2 * Math.PI) * colorPicker.wheelRadius
                                - width / 2
                            y: colorPicker.centerY
                                + Math.sin(root.pickerHue * 2 * Math.PI) * colorPicker.wheelRadius
                                - height / 2
                            color: "transparent"
                            border.width: 2
                            border.color: "white"
                        }

                        Rectangle {
                            width: 13
                            height: 13
                            radius: width / 2
                            x: colorPicker.centerX - colorPicker.squareSize / 2
                                + root.pickerSaturation * colorPicker.squareSize - width / 2
                            y: colorPicker.centerY - colorPicker.squareSize / 2
                                + (1 - root.pickerValue) * colorPicker.squareSize - height / 2
                            color: "transparent"
                            border.width: 2
                            border.color: "white"
                        }
                    }

                    RowLayout {
                        visible: root.lightingEnabled !== false
                        Layout.alignment: Qt.AlignHCenter
                        enabled: root.lightingWritable && root.canApplyLightingColor()
                        spacing: Kirigami.Units.smallSpacing

                        Rectangle {
                            Layout.preferredWidth: 32
                            Layout.preferredHeight: 32
                            radius: Kirigami.Units.cornerRadius
                            color: root.pickerColor
                            border.width: 1
                            border.color: Kirigami.Theme.textColor
                        }

                        PlasmaComponents.Label {
                            text: root.lightingTargetProfileKnown()
                                ? root.pickerHex().toUpperCase()
                                : "Nova cor sincronizada"
                            font.family: "monospace"
                        }

                    }
                }
            }
        }
    }

    Plasma5Support.DataSource {
        id: executable
        engine: "executable"

        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            if (sourceName !== root.command) {
                root.controlBusy = false
                const exitCode = Number(data["exit code"] ?? -1)
                const stderr = String(data.stderr ?? "").trim()
                if (exitCode === 0) {
                    root.actionFailed = false
                    root.errorMessage = ""
                    root.actionMessage = "Configuração aplicada"
                    clearAction.interval = 1800
                    clearAction.restart()
                    refreshAfterControl.restart()
                } else {
                    // Let the same color be retried after a failed write.
                    root.lastLiveLightingKey = ""
                    root.actionFailed = true
                    root.actionMessage = root.friendlyError(stderr, exitCode, "write")
                    clearAction.interval = 6000
                    clearAction.restart()
                    refreshAfterControl.restart()
                }
                root.flushLightingColor()
                return
            }
            root.finishRefresh(data)
        }
    }

    Plasma5Support.DataSource {
        id: cachedState
        engine: "executable"

        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            const exitCode = Number(data["exit code"] ?? -1)
            if (exitCode === 0) {
                root.finishRefresh(data)
                return
            }
            // The cache is unavailable only while the daemon starts/stops or
            // before it has observed the headset. Keep a direct HID read as a
            // compatibility fallback rather than polling it in normal use.
            executable.connectSource(root.command)
        }
    }

    Plasma5Support.DataSource {
        id: equalizerExecutable
        engine: "executable"

        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            const exitCode = Number(data["exit code"] ?? -1)
            const stdout = String(data.stdout ?? "").trim()
            const stderr = String(data.stderr ?? "").trim()
            if (sourceName === root.equalizerCommand) {
                root.equalizerUpdating = false
                if (exitCode !== 0) {
                    root.equalizerPipeWireActive = false
                    root.equalizerServiceActive = null
                    root.equalizerTargetConnected = null
                    root.equalizerDefaultSafe = null
                    root.equalizerTarget = ""
                    root.equalizerStatusError = stderr
                        || "Não foi possível consultar o PipeWire. Verifique se ele está em execução e tente novamente."
                    return
                }
                try {
                    const profile = JSON.parse(stdout)
                    const pipewire = profile.pipewire ?? {}
                    root.equalizerBypassed = profile.bypassed === true
                    // Bypassed, the live graph runs flat: show the saved bands.
                    const appliedBands = (root.equalizerBypassed ? null : pipewire.applied_bands)
                        ?? profile.applied_bands ?? profile.bands ?? []
                    // A status read while a `set` is in flight can predate it;
                    // the set's own answer updates the handles instead.
                    if (!root.equalizerBusy)
                        root.showConfirmedEqualizerBands(appliedBands.length === 10
                            ? appliedBands : profile.bands ?? [])
                    root.replaceEqualizerProfiles(profile.custom_profiles ?? [])
                    root.equalizerActiveProfileId = profile.active_profile_id === null
                        || profile.active_profile_id === undefined
                        ? "" : String(profile.active_profile_id)
                    root.equalizerPipeWireActive = (pipewire.active ?? profile.active) === true
                    root.equalizerServiceActive = pipewire.service_active
                        ?? profile.service_active ?? null
                    root.equalizerTargetConnected = pipewire.target_connected
                        ?? profile.target_connected ?? null
                    root.equalizerDefaultSafe = pipewire.default_safe
                        ?? profile.default_safe ?? null
                    root.equalizerTarget = String(pipewire.target_node_name
                        ?? profile.target_node_name ?? "")
                    root.equalizerStatusError = String(pipewire.error ?? profile.error ?? "")
                } catch (error) {
                    root.equalizerPipeWireActive = false
                    root.equalizerServiceActive = null
                    root.equalizerTargetConnected = null
                    root.equalizerDefaultSafe = null
                    root.equalizerTarget = ""
                    root.equalizerStatusError = `Resposta inválida do equalizador: ${error}`
                }
                return
            }
            if (sourceName === root.equalizerSetCommand) {
                root.equalizerSetCommand = ""
                root.finishEqualizerSet(exitCode, stdout, stderr)
                return
            }
            root.equalizerBusy = false
            const action = root.pendingEqualizerAction
            root.pendingEqualizerAction = ""
            if (exitCode === 0) {
                root.actionFailed = false
                root.actionMessage = root.equalizerActionMessage(action)
                root.equalizerActionError = ""
                root.closeEqualizerProfileEditors()
                clearAction.interval = 1800
                clearAction.restart()
            } else {
                root.equalizerActionError = stderr
                    || root.equalizerActionFailureMessage(action)
                // A rejected name stays in the editor so it can be corrected,
                // but a pending deletion always has to be confirmed again.
                root.equalizerDeleteConfirmation = false
            }
            // Reload after every mutation, accepted or rejected, so neither the
            // selector nor the handles can stay detached from the confirmed
            // profile library and DSP state.
            root.refreshEqualizer()
        }
    }

    Plasma5Support.DataSource {
        id: spatialExecutable
        engine: "executable"

        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            root.spatialUpdating = false
            const exitCode = Number(data["exit code"] ?? -1)
            const stdout = String(data.stdout ?? "").trim()
            const stderr = String(data.stderr ?? "").trim()
            if (exitCode !== 0) {
                root.spatialEnabled = false
                root.spatialMode = "off"
                root.spatialReady = false
                root.spatialDatasetValid = false
                root.spatialActive = false
                root.spatialServiceActive = false
                root.spatialError = stderr || "Não foi possível consultar o estado espacial"
                return
            }
            try {
                const result = JSON.parse(stdout)
                root.spatialEnabled = result.enabled === true
                root.spatialMode = String(result.mode ?? "off")
                root.spatialListening = String(result.listening ?? "game")
                if (result.sound && typeof result.sound === "object")
                    root.spatialSound = result.sound
                root.spatialActive = result.active === true
                root.spatialProcessingMode = String(result.processing_mode ?? (root.spatialEnabled ? "binaural" : "bypass"))
                root.spatialServiceActive = result.service_active === true
                const capability = result.capability ?? {}
                root.spatialReady = capability.ready === true
                const dataset = capability.dataset ?? {}
                root.spatialDatasetValid = result.dataset_valid === true || dataset.valid === true
                // A healthy status is authoritative. Clear any transient
                // startup error as soon as the backend reports recovery.
                root.spatialError = result.active === true
                    ? ""
                    : (root.spatialEnabled
                        ? String(result.error ?? capability.error ?? "")
                        : "")
            } catch (error) {
                root.spatialEnabled = false
                root.spatialMode = "off"
                root.spatialReady = false
                root.spatialDatasetValid = false
                root.spatialActive = false
                root.spatialServiceActive = false
                root.spatialError = `Resposta inválida do estado espacial: ${error}`
            }
        }
    }

    Plasma5Support.DataSource {
        id: spatialControlExecutable
        engine: "executable"

        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            root.spatialBusy = false
            const exitCode = Number(data["exit code"] ?? -1)
            const stderr = String(data.stderr ?? "").trim()
            if (exitCode !== 0)
                root.spatialError = stderr || "Não foi possível alterar o áudio espacial"
            root.refreshSpatial()
        }
    }

    DBus.SignalWatcher {
        busType: DBus.BusType.Session
        service: "org.jambalinux.soniccore.State"
        path: "/org/jambalinux/soniccore/State"
        iface: "org.jambalinux.soniccore.State"
        enabled: true

        function dbusChanged() {
            root.refresh()
        }
    }

    DBus.DBusServiceWatcher {
        id: daemonWatcher
        busType: DBus.BusType.Session
        watchedService: "org.jambalinux.soniccore.State"

        onRegisteredChanged: {
            if (registered) root.refresh()
        }
    }

    Plasma5Support.DataSource {
        id: openRgbSource
        engine: "executable"
        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            root.applyOpenRgbResult(sourceName, data)
        }
    }

    Plasma5Support.DataSource {
        id: wallpaperSource
        engine: "executable"
        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            if (Number(data["exit code"] ?? -1) !== 0) return
            try {
                const path = JSON.parse(String(data.stdout ?? "")).path
                if (typeof path === "string" && path.length > 0)
                    root.wallpaperUrl = "file://" + path
            } catch (error) {
            }
        }
    }

    Plasma5Support.DataSource {
        id: outputSource
        engine: "executable"
        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            if (Number(data["exit code"] ?? -1) !== 0) return
            try {
                const result = JSON.parse(String(data.stdout ?? ""))
                const current = (result.outputs ?? []).find(output => output.name === result.effective)
                root.outputIsQuantum = !current || current.quantum_game === true
                root.outputIcon = current?.icon ?? "audio-headphones"
                root.outputLabel = current?.description ?? ""
            } catch (error) {
                root.outputIsQuantum = true
            }
        }
    }

    // The output can change whenever the system default changes.
    Timer {
        interval: 5000
        repeat: true
        running: root.expanded
        triggeredOnStart: true
        onTriggered: outputSource.connectSource(root.soniccoreCommand(["output", "--format", "json"]))
    }


    // Measured battery estimate, refreshed while the popup is open.
    Plasma5Support.DataSource {
        id: powerSource
        engine: "executable"
        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            if (Number(data["exit code"] ?? -1) !== 0) return
            try {
                const power = JSON.parse(String(data.stdout ?? ""))
                const hours = Number(power?.history?.hours_left)
                root.hoursLeft = Number.isFinite(hours) ? hours : -1
                root.powerSaving = String(power?.saving ?? "")
            } catch (error) {
                root.hoursLeft = -1
            }
        }
    }

    Timer {
        interval: 60000
        repeat: true
        running: root.expanded
        triggeredOnStart: true
        onTriggered: powerSource.connectSource(root.soniccoreCommand(["power", "--format", "json"]))
    }

    // While the lighting section is open, follow who controls the lights.
    // Writes made by the CLI do not emit the daemon's D-Bus signal, so this
    // cheap cache read is what notices OpenRGB starting or stopping.
    Timer {
        interval: 2000
        repeat: true
        running: root.expanded
        triggeredOnStart: true
        onTriggered: {
            root.nowMs = Date.now()
            root.refresh()
        }
    }

    // At most one color write every 150 ms while dragging; the last color is
    // always sent once the drag stops.
    Timer {
        id: lightingColorThrottle
        interval: 150
        repeat: false
        onTriggered: root.flushLightingColor()
    }

    Timer {
        id: refreshAfterControl
        interval: 300
        repeat: false
        onTriggered: root.refresh()
    }

    // Full status reload after a burst of slider changes: the `set` answers
    // already moved the handles, this only re-syncs profiles and health.
    Timer {
        id: equalizerFullRefresh
        interval: 1000
        repeat: false
        onTriggered: {
            if (!root.equalizerBusy && Object.keys(root.pendingEqualizerBands).length === 0)
                root.refreshEqualizer()
        }
    }

    Timer {
        id: clearAction
        interval: 1800
        repeat: false
        onTriggered: {
            root.actionMessage = ""
            root.actionFailed = false
        }
    }

    Timer {
        interval: 300000
        repeat: true
        running: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    Timer {
        interval: (root.spatialEnabled && !root.spatialActive) || root.spatialMixPending ? 1000 : 15000
        repeat: true
        running: root.expanded
        triggeredOnStart: true
        onTriggered: root.refreshSpatial()
    }
}
