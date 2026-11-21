import QtQuick
import QtQuick.Effects
import QtQuick.Shapes

import org.kde.kirigami as Kirigami

import "palette.js" as Palette

// Background of the "Vivo" popup.
// - "aurora": three to five soft light blobs in the headset lights' colors drifting
//   slowly; the bass makes them swell.
// - "glass": the desktop wallpaper, downscaled and blurred once (the blurred
//   texture is cached; only a slow zoom moves it), dimmed for legibility,
//   with a glow in the lights' colors that pulses with the bass.
// Everything stops with the popup closed or with reduced animations.
Item {
    id: background

    property string style: "aurora"     // "aurora", "glass", "particles", "waves" or "none"
    property int intensity: 1           // 0 subtle, 1 medium, 2 intense
    property bool reactive: true        // follow the bass
    property bool running: false
    property real bass: 0               // 0..1 from the live meter
    property color accentA: Kirigami.Theme.highlightColor
    property color accentB: Kirigami.Theme.highlightColor
    // The headset lights' colors in order; empty uses accentA/accentB.
    property var lightPalette: []

    function paletteAt(position) {
        return Palette.at(lightPalette, position, mix(accentA, accentB, position))
    }
    property url wallpaper

    readonly property bool animate: Kirigami.Units.longDuration > 1
    readonly property real strength: [0.22, 0.38, 0.56][Math.max(0, Math.min(2, intensity))]
    readonly property int blobCount: Math.max(3, Math.min(5, lightPalette.length))
    property real t: 0
    property real pulse: 0
    // Accumulated motion for the shader styles: sound speeds them up
    // without making them jump.
    property real travel: 0
    property real phase: 0

    function tint(color, alpha) {
        return Qt.rgba(color.r, color.g, color.b, alpha)
    }

    function mix(first, second, amount) {
        return Qt.rgba(first.r + (second.r - first.r) * amount,
                       first.g + (second.g - first.g) * amount,
                       first.b + (second.b - first.b) * amount, 1)
    }

    // 30 frames per second is plenty for slow drifting light and keeps the
    // cost low on high refresh-rate screens.
    Timer {
        interval: 33
        repeat: true
        running: background.running && background.animate && background.visible
            && background.style !== "none"
        onTriggered: {
            background.t += 0.033
            const target = background.reactive ? background.bass : 0
            // Fast attack, slow release: beats punch, then fade.
            const rate = target > background.pulse ? 0.45 : 0.08
            background.pulse += (target - background.pulse) * rate
            background.travel += 0.033 * (0.35 + 1.6 * background.pulse)
            background.phase += 0.033 * (0.9 + 2.5 * background.pulse)
        }
    }

    onRunningChanged: if (!running) pulse = 0

    // Everything is clipped to the popup's rounded shape.
    Item {
        id: canvas
        anchors.fill: parent
        layer.enabled: background.style !== "none"
        // Half-float layers: 8-bit ones show banding in soft gradients,
        // which an HDR screen makes very visible.
        layer.format: ShaderEffectSource.RGBA16F
        layer.smooth: true
        layer.effect: MultiEffect {
            maskEnabled: true
            maskSource: roundedMask
        }

        // ---- Shader styles: drawn per pixel on the GPU, perfectly smooth ----
        ShaderEffect {
            visible: background.style === "particles"
            anchors.fill: parent
            blending: true
            property real travel: background.travel
            property real pulse: background.pulse
            property real strength: background.strength
            property real aspect: width / Math.max(1, height)
            property color color0: background.paletteAt(0)
            property color color1: background.paletteAt(0.25)
            property color color2: background.paletteAt(0.5)
            property color color3: background.paletteAt(0.75)
            property color color4: background.paletteAt(1)
            fragmentShader: Qt.resolvedUrl("../shaders/particles.frag.qsb")
        }

        ShaderEffect {
            visible: background.style === "waves"
            anchors.fill: parent
            blending: true
            property real phase: background.phase
            property real pulse: background.pulse
            property real strength: background.strength
            property color color0: background.paletteAt(0)
            property color color1: background.paletteAt(0.25)
            property color color2: background.paletteAt(0.5)
            property color color3: background.paletteAt(0.75)
            property color color4: background.paletteAt(1)
            fragmentShader: Qt.resolvedUrl("../shaders/waves.frag.qsb")
        }

        // ---- Glass: the wallpaper, blurred once and cached ----
        Image {
            id: wallpaperImage
            visible: background.style === "glass" && status === Image.Ready
            anchors.centerIn: parent
            width: parent.width
            height: parent.height
            source: background.style === "glass" ? background.wallpaper : ""
            // Decoded at the popup's own size: a small texture stretched
            // several times looks blocky even after blurring.
            sourceSize.height: Math.max(1, Math.ceil(height * 1.2))
            fillMode: Image.PreserveAspectCrop
            smooth: true
            mipmap: true
            asynchronous: true
            cache: true
            layer.enabled: visible
            layer.format: ShaderEffectSource.RGBA16F
            layer.smooth: true
            layer.mipmap: true
            layer.effect: MultiEffect {
                blurEnabled: true
                blur: 1.0
                blurMax: 64
                blurMultiplier: 0.6
                saturation: 0.15
            }
            // Moving the cached texture costs nothing; re-blurring would.
            scale: 1.12 + 0.03 * Math.sin(background.t * 0.05)
        }

        Rectangle {
            visible: background.style === "glass"
            anchors.fill: parent
            color: Kirigami.Theme.backgroundColor
            opacity: 0.62 - 0.1 * background.intensity
        }

        // ---- Light: aurora blobs, or the glass's central glow ----
        // Each blob is drawn once into a cached texture; per frame only its
        // position, scale and opacity change, which the GPU does for free.
        // Changing gradient geometry every frame cost ~8% of a CPU core.
        Repeater {
            // One blob per light color (3 to 5), so every color shows.
            model: background.style === "aurora" ? background.blobCount
                : background.style === "glass" ? 1 : 0
            delegate: Item {
                id: blob
                required property int index
                readonly property bool glow: background.style === "glass"
                readonly property real baseRadius: Math.max(canvas.width, canvas.height)
                    * (glow ? 0.35 : 0.45)
                readonly property real cx: glow
                    ? canvas.width / 2
                    : canvas.width * (0.5 + 0.38 * Math.sin(background.t * 0.07 + index * 2.1))
                readonly property real cy: glow
                    ? canvas.height * 0.22
                    : canvas.height * (0.45 + 0.32 * Math.cos(background.t * 0.05 + index * 1.3))
                readonly property color tone: glow
                    ? background.paletteAt(0.5)
                    : background.paletteAt(index / Math.max(1, background.blobCount - 1))

                width: baseRadius * 2
                height: baseRadius * 2
                x: cx - baseRadius
                y: cy - baseRadius
                scale: glow ? 1 + 0.7 * background.pulse : 1 + 0.4 * background.pulse
                opacity: Math.min(0.85, glow
                    ? background.strength * (0.25 + 0.9 * background.pulse)
                    : background.strength * (0.7 + 0.5 * background.pulse))

                Shape {
                    anchors.fill: parent
                    layer.enabled: true
                    layer.smooth: true
                    layer.format: ShaderEffectSource.RGBA16F

                    ShapePath {
                        strokeColor: "transparent"
                        fillGradient: RadialGradient {
                            centerX: blob.baseRadius
                            centerY: blob.baseRadius
                            centerRadius: blob.baseRadius
                            focalX: blob.baseRadius
                            focalY: blob.baseRadius
                            // A Gaussian-like falloff: no visible rings.
                            GradientStop { position: 0.0; color: blob.tone }
                            GradientStop { position: 0.2; color: background.tint(blob.tone, 0.82) }
                            GradientStop { position: 0.4; color: background.tint(blob.tone, 0.5) }
                            GradientStop { position: 0.6; color: background.tint(blob.tone, 0.22) }
                            GradientStop { position: 0.8; color: background.tint(blob.tone, 0.06) }
                            GradientStop { position: 1.0; color: background.tint(blob.tone, 0) }
                        }
                        startX: 0
                        startY: 0
                        PathLine { x: blob.width; y: 0 }
                        PathLine { x: blob.width; y: blob.height }
                        PathLine { x: 0; y: blob.height }
                        PathLine { x: 0; y: 0 }
                    }
                }
            }
        }
    }

    Rectangle {
        id: roundedMask
        anchors.fill: parent
        radius: Kirigami.Units.cornerRadius * 2
        visible: false
        layer.enabled: true
    }
}
