import QtQuick
import QtQuick.Layouts
import QtQml.Models

import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.plasma5support as Plasma5Support
import org.kde.plasma.plasmoid
import org.kde.plasma.workspace.dbus as DBus

PlasmoidItem {
    id: root

    readonly property string command: "/bin/sh -lc \"$HOME/.cargo/bin/soniccore status --format json\""
    readonly property string cachedCommand: "/bin/sh -lc \"$HOME/.cargo/bin/soniccore cached-status --format json\""
    readonly property string equalizerCommand: "/bin/sh -lc \"$HOME/.cargo/bin/soniccore equalizer status --format json\""
    readonly property string spatialCommand: "/bin/sh -lc \"$HOME/.cargo/bin/soniccore spatial status --format json\""
    readonly property url equipmentImage: Qt.resolvedUrl("../images/jbl-quantum-810.png")
    property int batteryPercent: -1
    property bool charging: false
    property string rawFeature: ""
    property string errorMessage: ""
    property string actionMessage: ""
    property string openSection: ""
    property string ambientMode: "unknown"
    property var headsetConnected: null
    property string microphoneState: "unknown"
    property string sidetoneLevel: "unknown"
    property var lightingEnabled: null
    property string lightingColor: "unknown"
    property string logoColor: "unknown"
    property string ringColor: "unknown"
    property var logoColors: ["#33ffcc", "#33ffcc", "#33ffcc", "#33ffcc", "#33ffcc"]
    property var ringColors: ["#33ffcc", "#33ffcc", "#33ffcc", "#33ffcc", "#33ffcc"]
    property string logoEffect: "solid"
    property string ringEffect: "solid"
    property string logoSpeed: "0.5"
    property string ringSpeed: "0.5"
    property string lightingTarget: "both"
    property int lightingSegment: -1
    property real pickerHue: 0
    property real pickerSaturation: 1
    property real pickerValue: 1
    readonly property color pickerColor: Qt.hsva(pickerHue, pickerSaturation, pickerValue, 1)
    property int gameChatValue: -1
    property bool controlBusy: false
    property bool updating: false
    property bool equalizerUpdating: false
    property bool equalizerBusy: false
    property int equalizerVisualRevision: 0
    property bool equalizerPipeWireActive: false
    property var equalizerServiceActive: null
    property var equalizerTargetConnected: null
    property var equalizerDefaultSafe: null
    property string equalizerTarget: ""
    property string equalizerError: ""
    property string pendingEqualizerAction: ""
    property int pendingEqualizerFrequency: 0
    property real pendingEqualizerGain: 0
    property bool spatialUpdating: false
    property bool spatialEnabled: false
    property string spatialMode: "off"
    property bool spatialReady: false
    property string spatialError: ""
    readonly property var equalizerFrequencies: [
        31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000
    ]
    readonly property bool deviceAvailable: batteryPercent >= 0
    readonly property bool daemonAvailable: daemonWatcher.registered
    readonly property color batteryColor: headsetConnected === false
        ? Kirigami.Theme.disabledTextColor
        : charging
        ? "#22d3ee"
        : batteryPercent < 0
        ? Kirigami.Theme.disabledTextColor
            : batteryPercent >= 60
            ? "#35c759"
            : batteryPercent >= 30 ? "#f5c542" : "#ff453a"

    ListModel {
        id: equalizerBandModel
    }

    Plasmoid.icon: "audio-headphones"
    Plasmoid.status: batteryPercent >= 0 ? PlasmaCore.Types.ActiveStatus : PlasmaCore.Types.PassiveStatus

    function refresh() {
        if (updating) {
            return
        }
        updating = true
        errorMessage = ""
        cachedState.connectSource(cachedCommand)
    }

    function runControl(feature, value) {
        if (controlBusy) return
        controlBusy = true
        clearAction.stop()
        actionMessage = "Aplicando…"
        const controlCommand = `/bin/sh -lc "$HOME/.cargo/bin/soniccore set ${feature} '${value}'"`
        executable.connectSource(controlCommand)
    }

    function refreshEqualizer() {
        if (equalizerUpdating) return
        equalizerUpdating = true
        equalizerExecutable.connectSource(equalizerCommand)
    }

    // Read-only: this only queries the experimental spatial/binaural gate and
    // capability preflight. It never enables, disables, or configures spatial
    // audio, and the widget exposes no control that could.
    function refreshSpatial() {
        if (spatialUpdating) return
        spatialUpdating = true
        spatialExecutable.connectSource(spatialCommand)
    }

    function spatialStatusText() {
        if (!root.spatialEnabled)
            return "Desativado por padrão (experimental)"
        if (!root.spatialReady)
            return "Ativado na configuração · capacidade indisponível"
        return `Ativado na configuração · modo ${root.spatialMode} · sem processamento de áudio`
    }

    function setEqualizerBand(frequencyHz, gainDb) {
        if (equalizerBusy || equalizerUpdating) return
        equalizerBusy = true
        const normalizedGain = Math.round(Number(gainDb) * 10) / 10
        pendingEqualizerAction = "set"
        pendingEqualizerFrequency = frequencyHz
        pendingEqualizerGain = normalizedGain
        const equalizerSetCommand = `/bin/sh -lc "$HOME/.cargo/bin/soniccore equalizer set ${frequencyHz} ${normalizedGain}"`
        equalizerExecutable.connectSource(equalizerSetCommand)
    }

    function resetEqualizer() {
        if (equalizerBusy || equalizerUpdating) return
        equalizerBusy = true
        pendingEqualizerAction = "reset"
        equalizerExecutable.connectSource("/bin/sh -lc \"$HOME/.cargo/bin/soniccore equalizer reset\"")
    }

    function replaceEqualizerBands(bands) {
        const gainsByFrequency = {}
        for (let index = 0; index < bands.length; ++index) {
            const frequency = Number(bands[index].frequency_hz)
            const gain = Number(bands[index].gain_db)
            if (Number.isFinite(frequency) && Number.isFinite(gain))
                gainsByFrequency[frequency] = gain
        }

        equalizerBandModel.clear()
        for (let index = 0; index < equalizerFrequencies.length; ++index) {
            const frequency = equalizerFrequencies[index]
            equalizerBandModel.append({
                "frequencyHz": frequency,
                "gainDb": Number(gainsByFrequency[frequency] ?? 0)
            })
        }
        equalizerVisualRevision += 1
    }

    function updateEqualizerBands(frequencyHz, gainDb, reset) {
        for (let index = 0; index < equalizerBandModel.count; ++index) {
            const band = equalizerBandModel.get(index)
            let nextGain = Number(band.gainDb)
            if (reset) nextGain = 0
            else if (Number(band.frequencyHz) === frequencyHz) nextGain = gainDb
            equalizerBandModel.setProperty(index, "gainDb", nextGain)
        }
        // A revision forces every handle to resynchronize even when its model
        // value was already zero and ListModel therefore emitted no role change.
        equalizerVisualRevision += 1
    }

    function equalizerStatusText() {
        if (equalizerDefaultSafe === false)
            return "Automático · restaurando a saída padrão"
        if (equalizerError.length > 0)
            return "Automático · indisponível"
        if (equalizerPipeWireActive) {
            return equalizerTarget.length > 0
                ? `Automático · ativo na rota Game: ${equalizerTarget}`
                : "Automático · ativo na rota Game"
        }
        if (equalizerTargetConnected === false)
            return "Automático · aguardando a rota Game do headset"
        if (equalizerServiceActive === true)
            return "Automático · preparando a rota Game"
        if (equalizerServiceActive === false)
            return "Automático · serviço indisponível"
        return "Automático · verificando o PipeWire"
    }

    function equalizerProblemText() {
        if (equalizerDefaultSafe === false) {
            return "A saída virtual não pode ser a saída padrão. A restauração automática para a saída física está em andamento."
        }
        if (equalizerError.length > 0)
            return equalizerError
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
            return String(colors[lightingSegment] ?? "unknown")
        }
        if (lightingTarget === "logo") return logoColor
        if (lightingTarget === "ring") return ringColor
        return lightingColor
    }

    function lightingFeature() {
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

    function normalizeColor(value) {
        const presets = {
            "blue": "#0029ff", "cyan": "#33ffcc", "magenta": "#ff00cc",
            "red": "#ff2020", "green": "#20ff66", "white": "#ffffff"
        }
        return presets[value] ?? (/^#[0-9a-fA-F]{6}$/.test(value) ? value : "#33ffcc")
    }

    function loadPicker() {
        const hex = normalizeColor(selectedLightingColor())
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

    function friendlyError(message) {
        const normalized = message.toLowerCase()
        if (normalized.includes("not found") || normalized.includes("não encontrado"))
            return "Dongle USB desconectado"
        if (normalized.includes("permission denied")
                || normalized.includes("permissão negada")
                || normalized.includes("operation not permitted"))
            return "Sem permissão para acessar o headset"
        return "Não foi possível ler o headset"
    }

    function applyResult(data) {
        const exitCode = Number(data["exit code"] ?? -1)
        const stdout = String(data.stdout ?? "").trim()
        const stderr = String(data.stderr ?? "").trim()
        if (exitCode !== 0) {
            batteryPercent = -1
            charging = false
            headsetConnected = false
            microphoneState = "unknown"
            ambientMode = "unknown"
            errorMessage = friendlyError(stderr)
            return
        }
        try {
            const result = JSON.parse(stdout)
            const percentage = Number(result.battery_percent)
            if (!Number.isFinite(percentage) || percentage < 0 || percentage > 100) {
                throw new Error("percentual inválido")
            }
            batteryPercent = percentage
            charging = result.charging === true
            rawFeature = String(result.raw_feature ?? "")
            ambientMode = String(result.ambient_mode ?? "unknown")
            headsetConnected = result.headset_connected ?? null
            microphoneState = String(result.microphone ?? "unknown")
            sidetoneLevel = String(result.sidetone_level ?? "unknown")
            lightingEnabled = result.lighting_enabled ?? null
            lightingColor = String(result.lighting_color ?? "unknown")
            logoColor = String(result.logo_color ?? result.lighting_color ?? "unknown")
            ringColor = String(result.ring_color ?? result.lighting_color ?? "unknown")
            const logoFallback = normalizeColor(logoColor)
            const ringFallback = normalizeColor(ringColor)
            logoColors = result.logo_colors ?? [logoFallback, logoFallback, logoFallback, logoFallback, logoFallback]
            ringColors = result.ring_colors ?? [ringFallback, ringFallback, ringFallback, ringFallback, ringFallback]
            logoEffect = String(result.logo_effect ?? "solid")
            ringEffect = String(result.ring_effect ?? "solid")
            logoSpeed = String(result.logo_speed ?? "0.5")
            ringSpeed = String(result.ring_speed ?? "0.5")
            gameChatValue = result.game_chat_value === null ? -1 : Number(result.game_chat_value)
            errorMessage = ""
        } catch (error) {
            batteryPercent = -1
            charging = false
            errorMessage = `Resposta inválida: ${error}`
        }
    }

    compactRepresentation: MouseArea {
        id: compact

        implicitWidth: 34 + Kirigami.Units.smallSpacing
        implicitHeight: Math.max(compactLayout.implicitHeight, 18)
        Layout.minimumWidth: implicitWidth
        Layout.preferredWidth: implicitWidth
        Layout.minimumHeight: implicitHeight
        Layout.preferredHeight: implicitHeight
        onClicked: root.expanded = !root.expanded

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

    fullRepresentation: ColumnLayout {
        id: expandedRepresentation
        readonly property int equalizerColumns: width >= Kirigami.Units.gridUnit * 20 ? 10 : 5
        readonly property real requiredGridHeight: root.openSection === "lighting"
            ? 52 : root.openSection === "equalizer" ? 57 : root.openSection.length > 0 ? 32 : 25

        Layout.minimumWidth: Kirigami.Units.gridUnit * 19
        Layout.minimumHeight: Kirigami.Units.gridUnit * requiredGridHeight
        Layout.preferredWidth: Kirigami.Units.gridUnit * 21
        Layout.preferredHeight: Kirigami.Units.gridUnit * requiredGridHeight
        spacing: Kirigami.Units.smallSpacing

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
            text: "JBL Quantum 810 Wireless"
            opacity: 0.7
        }

        Image {
            source: root.equipmentImage
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
            Layout.alignment: Qt.AlignHCenter
            Layout.maximumHeight: implicitHeight
            text: root.batteryPercent >= 0 ? `${root.batteryPercent}%` : "Indisponível"
            color: root.batteryColor
            font.pixelSize: Kirigami.Units.gridUnit * 1.7
            font.bold: true
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
            color: root.errorMessage.length > 0
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

        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            Layout.maximumHeight: implicitHeight
            spacing: Kirigami.Units.smallSpacing
                PlasmaComponents.Button {
                    text: "Ambiente"
                icon.name: "audio-headphones-symbolic"
                enabled: root.deviceAvailable && !root.controlBusy
                checkable: true
                checked: root.openSection === "ambient"
                onClicked: root.openSection = checked ? "ambient" : ""
            }

            PlasmaComponents.Button {
                text: "Luzes"
                icon.source: Qt.resolvedUrl("../images/light-bulb.svg")
                enabled: root.deviceAvailable && !root.controlBusy
                checkable: true
                checked: root.openSection === "lighting"
                onClicked: {
                    root.openSection = checked ? "lighting" : ""
                    if (checked) root.loadPicker()
                }
            }

            PlasmaComponents.Button {
                text: "Retorno"
                icon.name: "microphone-sensitivity-high"
                enabled: root.deviceAvailable && !root.controlBusy
                checkable: true
                checked: root.openSection === "sidetone"
                onClicked: root.openSection = checked ? "sidetone" : ""
            }

            PlasmaComponents.Button {
                text: "Equalizador"
                icon.name: "view-filter"
                enabled: !root.equalizerBusy
                checkable: true
                checked: root.openSection === "equalizer"
                onClicked: {
                    root.openSection = checked ? "equalizer" : ""
                    if (checked) root.refreshEqualizer()
                }
            }

            PlasmaComponents.Button {
                text: "Espacial"
                icon.name: "audio-speakers-symbolic"
                checkable: true
                checked: root.openSection === "spatial"
                onClicked: {
                    root.openSection = checked ? "spatial" : ""
                    if (checked) root.refreshSpatial()
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: controlOptions.implicitHeight + Kirigami.Units.largeSpacing * 2
            visible: root.openSection.length > 0
            radius: Kirigami.Units.cornerRadius
            color: Kirigami.Theme.backgroundColor
            border.width: 1
            border.color: Kirigami.Theme.disabledTextColor

            ColumnLayout {
                id: controlOptions
                anchors.fill: parent
                anchors.margins: Kirigami.Units.largeSpacing
                spacing: Kirigami.Units.smallSpacing

                PlasmaComponents.Label {
                    Layout.alignment: Qt.AlignHCenter
                    text: root.openSection === "ambient"
                        ? "Controle de som ambiente"
                        : root.openSection === "lighting" ? "Iluminação"
                        : root.openSection === "equalizer" ? "Equalizador de saída"
                        : root.openSection === "spatial" ? "Áudio espacial / binaural (experimental)"
                        : "Retorno do microfone"
                    font.bold: true
                }

                PlasmaComponents.Label {
                    Layout.alignment: Qt.AlignHCenter
                    text: root.openSection === "ambient"
                        ? `Atual: ${root.ambientLabel(root.ambientMode)}`
                        : root.openSection === "lighting"
                            ? root.lightingEnabled === null ? "Aguardando estado" : root.lightingEnabled ? "Atual: ligada" : "Atual: desligada"
                            : root.openSection === "equalizer"
                                ? root.equalizerStatusText()
                            : root.openSection === "spatial"
                                ? root.spatialStatusText()
                            : `Atual: ${root.sidetoneLabel(root.sidetoneLevel)}`
                    opacity: 0.7
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "ambient"
                    enabled: root.deviceAvailable && !root.controlBusy
                    PlasmaComponents.Button { text: "Desligado"; checkable: true; checked: root.ambientMode === "off"; onClicked: root.runControl("ambient", "off") }
                    PlasmaComponents.Button { text: "ANC"; checkable: true; checked: root.ambientMode === "anc"; onClicked: root.runControl("ambient", "anc") }
                    PlasmaComponents.Button { text: "TalkThru"; checkable: true; checked: root.ambientMode === "talkthru"; onClicked: root.runControl("ambient", "talkthru") }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.openSection === "equalizer"
                    enabled: !root.equalizerBusy && !root.equalizerUpdating
                    spacing: Kirigami.Units.smallSpacing

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
                                    interval: 180
                                    repeat: false
                                    onTriggered: {
                                        if (!equalizerSlider.pressed) root.setEqualizerBand(
                                            equalizerBand.frequencyHz,
                                            equalizerSlider.value)
                                    }
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
                                    onMoved: equalizerCommit.restart()
                                    onPressedChanged: {
                                        if (!pressed) equalizerCommit.restart()
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
                        Layout.alignment: Qt.AlignHCenter
                        text: "Zerar bandas"
                        icon.name: "edit-clear"
                        onClicked: root.resetEqualizer()
                    }

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        text: root.equalizerPipeWireActive
                            ? "A rota Game é processada automaticamente; Chat, microfone e a saída padrão permanecem fora da cadeia."
                            : "O equalizador se conecta automaticamente quando a rota Game fica disponível; Chat e microfone permanecem fora da cadeia."
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        opacity: 0.7
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

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "lighting"
                    enabled: root.deviceAvailable && !root.controlBusy
                    PlasmaComponents.Button { text: "Ligar"; icon.source: Qt.resolvedUrl("../images/light-on.svg"); checkable: true; checked: root.lightingEnabled === true; onClicked: root.runControl("lighting", "on") }
                    PlasmaComponents.Button { text: "Desligar"; icon.source: Qt.resolvedUrl("../images/light-off.svg"); checkable: true; checked: root.lightingEnabled === false; onClicked: root.runControl("lighting", "off") }
                }

                PlasmaComponents.Label {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "lighting"
                    text: "Cor sólida personalizada"
                    opacity: 0.7
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "lighting"
                    enabled: root.deviceAvailable && !root.controlBusy

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
                    visible: root.openSection === "lighting"
                    text: "Efeito"
                    opacity: 0.7
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "lighting"
                    enabled: root.deviceAvailable && !root.controlBusy
                    spacing: Kirigami.Units.smallSpacing

                    PlasmaComponents.Button { text: "Respiração"; checkable: true; checked: root.selectedLightingEffect() === "breathing"; onClicked: root.runControl(root.lightingEffectFeature(), "breathing") }
                    PlasmaComponents.Button { text: "Sólido"; checkable: true; checked: root.selectedLightingEffect() === "solid"; onClicked: root.runControl(root.lightingEffectFeature(), "solid") }
                    PlasmaComponents.Button { text: "Onda"; checkable: true; checked: root.selectedLightingEffect() === "wave"; onClicked: root.runControl(root.lightingEffectFeature(), "wave") }
                    PlasmaComponents.Button { text: "Falha"; checkable: true; checked: root.selectedLightingEffect() === "glitch"; onClicked: root.runControl(root.lightingEffectFeature(), "glitch") }
                }

                PlasmaComponents.Label {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "lighting"
                    text: "Velocidade"
                    opacity: 0.7
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "lighting"
                    enabled: root.deviceAvailable && !root.controlBusy
                    spacing: Kirigami.Units.smallSpacing

                    PlasmaComponents.Button { text: "0,5×"; checkable: true; checked: root.selectedLightingSpeed() === "0.5"; onClicked: root.runControl(root.lightingSpeedFeature(), "0.5") }
                    PlasmaComponents.Button { text: "1×"; checkable: true; checked: root.selectedLightingSpeed() === "1.0"; onClicked: root.runControl(root.lightingSpeedFeature(), "1.0") }
                    PlasmaComponents.Button { text: "1,5×"; checkable: true; checked: root.selectedLightingSpeed() === "1.5"; onClicked: root.runControl(root.lightingSpeedFeature(), "1.5") }
                    PlasmaComponents.Button { text: "2×"; checkable: true; checked: root.selectedLightingSpeed() === "2.0"; onClicked: root.runControl(root.lightingSpeedFeature(), "2.0") }
                }

                PlasmaComponents.Label {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "lighting"
                    text: root.lightingSegment < 0
                        ? "Cor: todos os segmentos"
                        : `Cor: segmento ${root.lightingSegment + 1}`
                    opacity: 0.7
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "lighting"
                    enabled: root.deviceAvailable && !root.controlBusy
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
                            Layout.preferredWidth: 34
                            Layout.preferredHeight: 30
                            radius: Kirigami.Units.cornerRadius
                            color: root.lightingTarget === "ring"
                                ? root.ringColors[index] : root.logoColors[index]
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
                    id: colorPicker
                    Layout.alignment: Qt.AlignHCenter
                    Layout.preferredWidth: 220
                    Layout.preferredHeight: 220
                    visible: root.openSection === "lighting"
                    enabled: root.deviceAvailable && !root.controlBusy

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
                                svCanvas.requestPaint()
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
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "lighting"
                    enabled: root.deviceAvailable && !root.controlBusy
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
                        text: root.pickerHex().toUpperCase()
                        font.family: "monospace"
                    }

                    PlasmaComponents.Button {
                        text: "Aplicar"
                        icon.name: "dialog-ok-apply"
                        onClicked: root.runControl(root.lightingFeature(), root.pickerHex())
                    }
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.openSection === "sidetone"
                    enabled: root.deviceAvailable && !root.controlBusy
                    PlasmaComponents.Button { text: "Off"; checkable: true; checked: root.sidetoneLevel === "off"; onClicked: root.runControl("sidetone", "off") }
                    PlasmaComponents.Button { text: "Baixo"; checkable: true; checked: root.sidetoneLevel === "low"; onClicked: root.runControl("sidetone", "low") }
                    PlasmaComponents.Button { text: "Médio"; checkable: true; checked: root.sidetoneLevel === "medium"; onClicked: root.runControl("sidetone", "medium") }
                    PlasmaComponents.Button { text: "Alto"; checkable: true; checked: root.sidetoneLevel === "high"; onClicked: root.runControl("sidetone", "high") }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.openSection === "spatial"
                    spacing: Kirigami.Units.smallSpacing

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        text: "Recurso experimental e desativado por padrão. Este painel é somente leitura: nenhuma opção aqui liga áudio espacial/binaural."
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        opacity: 0.7
                    }

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        text: root.spatialReady
                            ? "Capacidade: componentes necessários detectados"
                            : "Capacidade: indisponível (ver soniccore spatial preflight)"
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        color: root.spatialReady
                            ? Kirigami.Theme.positiveTextColor
                            : Kirigami.Theme.neutralTextColor
                    }

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        visible: root.spatialError.length > 0
                        text: root.spatialError
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        color: Kirigami.Theme.negativeTextColor
                    }

                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        text: "Use a CLI `soniccore spatial` para consultar ou alterar a configuração fora deste widget."
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        opacity: 0.6
                    }
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
                    root.errorMessage = ""
                    root.actionMessage = "Configuração aplicada"
                    clearAction.restart()
                    refreshAfterControl.restart()
                } else {
                    root.errorMessage = root.friendlyError(stderr)
                    root.actionMessage = root.errorMessage
                }
                return
            }
            root.updating = false
            root.applyResult(data)
        }
    }

    Plasma5Support.DataSource {
        id: cachedState
        engine: "executable"

        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            const exitCode = Number(data["exit code"] ?? -1)
            if (exitCode === 0) {
                root.updating = false
                root.applyResult(data)
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
                    root.equalizerError = stderr
                        || "Não foi possível consultar o PipeWire. Verifique se ele está em execução e tente novamente."
                    return
                }
                try {
                    const profile = JSON.parse(stdout)
                    const pipewire = profile.pipewire ?? {}
                    const appliedBands = pipewire.applied_bands
                        ?? profile.applied_bands ?? profile.bands ?? []
                    root.replaceEqualizerBands(appliedBands.length === 10
                        ? appliedBands : profile.bands ?? [])
                    root.equalizerPipeWireActive = (pipewire.active ?? profile.active) === true
                    root.equalizerServiceActive = pipewire.service_active
                        ?? profile.service_active ?? null
                    root.equalizerTargetConnected = pipewire.target_connected
                        ?? profile.target_connected ?? null
                    root.equalizerDefaultSafe = pipewire.default_safe
                        ?? profile.default_safe ?? null
                    root.equalizerTarget = String(pipewire.target_node_name
                        ?? profile.target_node_name ?? "")
                    root.equalizerError = String(pipewire.error ?? profile.error ?? "")
                } catch (error) {
                    root.equalizerPipeWireActive = false
                    root.equalizerServiceActive = null
                    root.equalizerTargetConnected = null
                    root.equalizerDefaultSafe = null
                    root.equalizerTarget = ""
                    root.equalizerError = `Resposta inválida do equalizador: ${error}`
                }
                return
            }
            root.equalizerBusy = false
            if (exitCode === 0) {
                if (root.pendingEqualizerAction === "reset") {
                    root.updateEqualizerBands(0, 0, true)
                    root.actionMessage = "Bandas zeradas"
                } else if (root.pendingEqualizerAction === "set") {
                    root.updateEqualizerBands(root.pendingEqualizerFrequency,
                        root.pendingEqualizerGain, false)
                    root.actionMessage = "Equalizador atualizado"
                }
                root.pendingEqualizerAction = ""
                root.equalizerError = ""
                clearAction.restart()
                root.refreshEqualizer()
            } else {
                root.pendingEqualizerAction = ""
                root.equalizerError = stderr || "Não foi possível atualizar o equalizador"
                // Restore the canonical values after a rejected update so a
                // dragged handle never remains visually detached from DSP.
                root.refreshEqualizer()
            }
        }
    }

    // Read-only status query for the experimental spatial/binaural gate. This
    // DataSource only ever runs `soniccore spatial status --format json`; it
    // has no write/set counterpart and cannot enable anything.
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
                root.spatialError = stderr || "Não foi possível consultar o estado espacial"
                return
            }
            try {
                const result = JSON.parse(stdout)
                root.spatialEnabled = result.enabled === true
                root.spatialMode = String(result.mode ?? "off")
                const capability = result.capability ?? {}
                root.spatialReady = capability.ready === true
                root.spatialError = String(capability.error ?? "")
            } catch (error) {
                root.spatialEnabled = false
                root.spatialMode = "off"
                root.spatialReady = false
                root.spatialError = `Resposta inválida do estado espacial: ${error}`
            }
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

    Timer {
        id: refreshAfterControl
        interval: 300
        repeat: false
        onTriggered: root.refresh()
    }

    Timer {
        id: clearAction
        interval: 1800
        repeat: false
        onTriggered: root.actionMessage = ""
    }

    Timer {
        interval: 300000
        repeat: true
        running: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }
}
