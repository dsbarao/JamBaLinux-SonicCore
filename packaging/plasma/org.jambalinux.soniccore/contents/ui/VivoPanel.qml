import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import QtQuick.Shapes

import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.plasma5support as Plasma5Support
import org.kde.plasma.workspace.dbus as DBus

import "palette.js" as Palette

// The "Vivo" header: battery ring around the headset, the Game audio path
// with flowing links, and a live spectrum, all in the headset lights' colors.
// It only reads state: the spectrum comes from the on-demand D-Bus meter and
// the path from `soniccore audio-path`; nothing here writes to the device.
Item {
    id: panel

    // Animate and poll only while the popup is open with this theme.
    property bool running: false
    property real batteryPercent: -1
    property bool charging: false
    property var headsetConnected: null
    property color batteryColor: Kirigami.Theme.disabledTextColor
    property color accentA: Kirigami.Theme.highlightColor
    property color accentB: Kirigami.Theme.highlightColor
    // The headset lights' colors in order; empty uses accentA/accentB.
    property var lightPalette: []

    function paletteAt(position) {
        return Palette.at(lightPalette, position, mix(accentA, accentB, position))
    }
    property url equipmentImage
    property string pathCommand: ""
    property string hoursLeftText: ""
    property bool outputIsQuantum: true
    property string outputIcon: "audio-headphones"

    // Plasma's "Animation speed: Instant" sets durations to zero.
    readonly property bool animate: Kirigami.Units.longDuration > 1
    readonly property int bandCount: 32

    property var bands: []
    // Mean of the lowest bands (31-125 Hz), for the background's pulse.
    readonly property real bass: {
        if (!soundActive || bands.length < 6) return 0
        let sum = 0
        for (let index = 0; index < 6; ++index) sum += bands[index]
        return sum / 6
    }
    property bool soundActive: false
    property var path: null
    property bool framePending: false
    // One shared 25 Hz beat drives every moving part. Free-running QML
    // animations would redraw at the screen's refresh rate (165 Hz here) and
    // re-tessellate the dashed paths on every frame.
    property real dashPhase: 0
    property real chargePhase: 0

    implicitHeight: Kirigami.Units.gridUnit * 15

    function mix(first, second, amount) {
        return Qt.rgba(first.r + (second.r - first.r) * amount,
                       first.g + (second.g - first.g) * amount,
                       first.b + (second.b - first.b) * amount, 1)
    }

    function requestFrame() {
        if (framePending) return
        framePending = true
        const reply = DBus.SessionBus.asyncCall({
            service: "org.jambalinux.soniccore.Meter",
            path: "/org/jambalinux/soniccore/Meter",
            iface: "org.jambalinux.soniccore.Meter",
            member: "Frame",
            arguments: []
        })
        reply.finished.connect(function() {
            framePending = false
            if (reply.isError || !reply.values || reply.values.length < 3) {
                soundActive = false
                return
            }
            const raw = reply.values[0] ?? []
            const levels = []
            for (let index = 0; index < raw.length; ++index) {
                const entry = raw[index]
                const level = Number(entry && entry.value !== undefined ? entry.value : entry)
                levels.push(Number.isFinite(level) ? Math.max(0, Math.min(1, level)) : 0)
            }
            bands = levels
            soundActive = reply.values[2] === true
        })
    }

    function linkState(connected, expected) {
        if (connected)
            return pathFlowing() ? "flowing" : "idle"
        return expected ? "broken" : "off"
    }

    function pathFlowing() {
        return path !== null && path.stages.streams.running === true
            && path.stages.headset.running === true
    }

    // Spatial is optional: without it, apps feed the equalizer directly and
    // the first two segments are drawn as one pass-through.
    function segmentStates() {
        if (!path) return ["off", "off", "off"]
        const stages = path.stages
        const links = path.links
        const spatial = stages.spatial.present === true
        const equalizer = stages.equalizer.present === true
        const headset = stages.headset.present === true
        const appsIn = spatial ? links.streams_spatial === true : links.streams_equalizer === true
        const middle = spatial ? links.spatial_equalizer === true : links.streams_equalizer === true
        return [
            linkState(appsIn, false),
            linkState(middle, spatial && equalizer),
            linkState(links.equalizer_headset === true, equalizer && headset)
        ]
    }

    function segmentColor(state, index) {
        if (state === "broken") return Kirigami.Theme.negativeTextColor
        if (state === "off") return Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g,
                                            Kirigami.Theme.textColor.b, 0.18)
        return paletteAt((index + 0.5) / 3)
    }

    Timer {
        interval: 40
        repeat: true
        running: panel.running
        triggeredOnStart: true
        onTriggered: {
            panel.requestFrame()
            if (panel.animate && panel.pathFlowing())
                panel.dashPhase = (panel.dashPhase + 0.4) % 5
            if (panel.animate && panel.charging)
                panel.chargePhase = (panel.chargePhase + 0.11) % (2 * Math.PI)
        }
    }

    Timer {
        interval: 2000
        repeat: true
        running: panel.running && panel.pathCommand.length > 0
        triggeredOnStart: true
        onTriggered: pathSource.connectSource(panel.pathCommand)
    }

    onRunningChanged: if (!running) {
        bands = []
        soundActive = false
    }

    Plasma5Support.DataSource {
        id: pathSource
        engine: "executable"
        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            if (Number(data["exit code"] ?? -1) !== 0) {
                panel.path = null
                return
            }
            try {
                const parsed = JSON.parse(String(data.stdout ?? ""))
                panel.path = parsed && parsed.stages && parsed.links ? parsed : null
            } catch (error) {
                panel.path = null
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Kirigami.Units.smallSpacing

        // Battery ring around the headset.
        Item {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: Kirigami.Units.gridUnit * 6.5
            Layout.preferredHeight: Kirigami.Units.gridUnit * 6.5

            Shape {
                id: ring
                anchors.fill: parent
                // Analytic antialiasing: the glow puts the ring in a layer,
                // and layers get no multisampling, so the default renderer
                // left jagged edges.
                preferredRendererType: Shape.CurveRenderer
                readonly property real radius: width / 2 - 6
                layer.enabled: true
                layer.effect: MultiEffect {
                    shadowEnabled: true
                    shadowColor: panel.batteryColor
                    shadowBlur: 1.0
                    shadowHorizontalOffset: 0
                    shadowVerticalOffset: 0
                    shadowOpacity: glowPulse.value
                    blurMax: 32
                }

                ShapePath {
                    strokeColor: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g,
                                         Kirigami.Theme.textColor.b, 0.12)
                    strokeWidth: 6
                    fillColor: "transparent"
                    PathAngleArc {
                        centerX: ring.width / 2
                        centerY: ring.height / 2
                        radiusX: ring.radius
                        radiusY: ring.radius
                        startAngle: 0
                        sweepAngle: 360
                    }
                }
                ShapePath {
                    strokeColor: panel.batteryColor
                    strokeWidth: 6
                    capStyle: ShapePath.RoundCap
                    fillColor: "transparent"
                    PathAngleArc {
                        centerX: ring.width / 2
                        centerY: ring.height / 2
                        radiusX: ring.radius
                        radiusY: ring.radius
                        startAngle: -90
                        sweepAngle: 360 * Math.max(0, panel.batteryPercent) / 100
                    }
                }
            }

            // Slow breathing while charging (from the shared beat); steady otherwise.
            QtObject {
                id: glowPulse
                readonly property real value: panel.charging && panel.animate
                    ? 0.65 + 0.35 * Math.sin(panel.chargePhase) : 0.9
            }

            Kirigami.Icon {
                anchors.centerIn: parent
                visible: !panel.outputIsQuantum
                width: parent.width * 0.5
                height: width
                source: panel.outputIcon
            }

            Image {
                anchors.centerIn: parent
                visible: panel.outputIsQuantum
                width: parent.width * 0.62
                height: width
                source: panel.equipmentImage
                // The photo is 1254 px; scale it once, with a high-quality
                // filter, to twice the size it is shown at, instead of
                // squeezing the full texture on every frame.
                sourceSize.width: Math.ceil(width * 2)
                sourceSize.height: Math.ceil(height * 2)
                fillMode: Image.PreserveAspectFit
                smooth: true
                mipmap: true
                opacity: panel.headsetConnected === false ? 0.45 : 1
            }

        }

        PlasmaComponents.Label {
            Layout.alignment: Qt.AlignHCenter
            // The ring is always the headset's battery; say so when the
            // header shows another output.
            text: (panel.outputIsQuantum ? "" : "Fone ") + (panel.batteryPercent >= 0
                ? `${panel.batteryPercent}%${panel.charging ? " ⚡" : ""}` : "—")
            color: panel.batteryColor
            font.bold: true
            font.pixelSize: Kirigami.Units.gridUnit * 1.1
        }

        PlasmaComponents.Label {
            Layout.alignment: Qt.AlignHCenter
            visible: panel.hoursLeftText.length > 0
            text: panel.hoursLeftText
            opacity: 0.7
            font.pixelSize: Kirigami.Units.gridUnit * 0.7
        }

        // Audio path: Apps -> Espacial -> EQ -> Fone.
        Item {
            id: pathRow
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 3
            readonly property var states: panel.segmentStates()
            readonly property var labels: ["Apps", "Espacial", "EQ",
                                           panel.outputIsQuantum ? "Fone" : "Saída"]
            readonly property var icons: ["applications-multimedia", "audio-speakers-symbolic",
                                          "view-media-equalizer", panel.outputIcon]
            readonly property real nodeSize: Kirigami.Units.gridUnit * 1.6
            readonly property real lineY: nodeSize / 2

            // Room for the end labels so they are never clipped.
            readonly property real edge: Kirigami.Units.gridUnit * 1.75

            function nodeX(index) {
                return edge + index * (width - 2 * edge) / 3
            }

            function stagePresent(index) {
                if (!panel.path) return false
                const stages = panel.path.stages
                return [stages.streams.count > 0, stages.spatial.present, stages.equalizer.present,
                        stages.headset.present][index] === true
            }

            Repeater {
                model: 3
                delegate: Shape {
                    required property int index
                    preferredRendererType: Shape.CurveRenderer
                    readonly property string linkState: pathRow.states[index]
                    readonly property color tint: panel.segmentColor(linkState, index)
                    x: pathRow.nodeX(index)
                    y: pathRow.lineY - 4
                    width: pathRow.nodeX(index + 1) - pathRow.nodeX(index)
                    height: 8
                    layer.enabled: linkState === "flowing"
                    layer.effect: MultiEffect {
                        shadowEnabled: true
                        shadowColor: tint
                        shadowBlur: 0.9
                        shadowHorizontalOffset: 0
                        shadowVerticalOffset: 0
                        blurMax: 28
                    }

                    ShapePath {
                        id: segment
                        strokeColor: tint
                        strokeWidth: linkState === "flowing" ? 3 : 2
                        fillColor: "transparent"
                        strokeStyle: linkState === "flowing" || linkState === "broken"
                            ? ShapePath.DashLine : ShapePath.SolidLine
                        dashPattern: [3, 2]
                        startX: 0
                        startY: 4
                        PathLine { x: width; y: 4 }
                        // The dashes run toward the headset while sound flows.
                        dashOffset: linkState === "flowing" ? 5 - panel.dashPhase : 0
                    }
                }
            }

            Repeater {
                model: 4
                delegate: Item {
                    required property int index
                    readonly property bool present: pathRow.stagePresent(index)
                    readonly property color tint: present
                        ? panel.paletteAt(index / 3)
                        : Kirigami.Theme.disabledTextColor
                    x: pathRow.nodeX(index) - width / 2
                    width: Kirigami.Units.gridUnit * 3.5
                    height: pathRow.height

                    Rectangle {
                        id: node
                        anchors.horizontalCenter: parent.horizontalCenter
                        width: pathRow.nodeSize
                        height: width
                        radius: width / 2
                        color: Kirigami.Theme.backgroundColor
                        border.width: 2
                        border.color: tint
                        opacity: present ? 1 : 0.5
                        layer.enabled: present && panel.pathFlowing()
                        layer.effect: MultiEffect {
                            shadowEnabled: true
                            shadowColor: tint
                            shadowBlur: 1.0
                            shadowHorizontalOffset: 0
                            shadowVerticalOffset: 0
                            blurMax: 32
                        }

                        Kirigami.Icon {
                            anchors.centerIn: parent
                            width: parent.width * 0.6
                            height: width
                            source: pathRow.icons[index]
                            color: tint
                            isMask: true
                        }
                    }

                    PlasmaComponents.Label {
                        anchors.horizontalCenter: parent.horizontalCenter
                        anchors.top: node.bottom
                        anchors.topMargin: 2
                        text: pathRow.labels[index]
                        font.pixelSize: Kirigami.Units.gridUnit * 0.65
                        opacity: present ? 0.85 : 0.45
                    }
                }
            }
        }

        // Live spectrum.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: Kirigami.Units.gridUnit * 3

            Row {
                id: bars
                anchors.fill: parent
                spacing: 2
                layer.enabled: panel.soundActive
                layer.effect: MultiEffect {
                    shadowEnabled: true
                    shadowColor: panel.paletteAt(0.5)
                    shadowBlur: 0.9
                    shadowHorizontalOffset: 0
                    shadowVerticalOffset: 0
                    blurMax: 28
                }

                Repeater {
                    model: panel.bandCount
                    delegate: Item {
                        required property int index
                        readonly property real level: index < panel.bands.length ? panel.bands[index] : 0
                        width: (bars.width - bars.spacing * (panel.bandCount - 1)) / panel.bandCount
                        height: bars.height

                        Rectangle {
                            anchors.bottom: parent.bottom
                            width: parent.width
                            radius: Math.min(width / 2, 2)
                            // A thin floor keeps the shape visible in silence.
                            height: Math.max(2, parent.height * level)
                            color: panel.paletteAt(index / (panel.bandCount - 1))
                            opacity: panel.soundActive ? 0.95 : 0.3
                        }
                    }
                }
            }

            PlasmaComponents.Label {
                anchors.centerIn: parent
                visible: !panel.soundActive
                text: "Nada tocando no Game"
                opacity: 0.5
                font.pixelSize: Kirigami.Units.gridUnit * 0.7
            }
        }
    }
}
