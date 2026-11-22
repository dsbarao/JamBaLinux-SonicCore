import QtCore
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import Qt.labs.platform as Platform

import org.kde.kirigami as Kirigami

import org.jambalinux.soniccore as Sonic

import "contents/ui"

// JamBaLinux Sonic Core: the interface (contents/ui/SonicCore.qml) in its own
// window, with a tray icon showing the battery where the desktop has a tray.
// Closing the window keeps it in the tray; "Sair" quits. Without a tray
// (GNOME without the AppIndicator extension) closing the window quits.
//
// Opened by the soniccore-app launcher (packaging/app/launcher), which
// provides the org.jambalinux.soniccore helpers; from the repository:
// `SONICCORE_APP_DIR=packaging/app soniccore-app [--tray]`.
Kirigami.ApplicationWindow {
    id: window

    title: "JamBaLinux Sonic Core"
    width: Kirigami.Units.gridUnit * 68
    height: Kirigami.Units.gridUnit * 46
    minimumWidth: Kirigami.Units.gridUnit * 25
    minimumHeight: Kirigami.Units.gridUnit * 24
    // At login (autostart passes --tray) only the tray icon appears.
    readonly property bool startedInTray: Qt.application.arguments.includes("--tray")
    visible: !startedInTray

    // The app's own settings, with the same names the widget uses.
    Settings {
        id: settings
        location: StandardPaths.writableLocation(StandardPaths.ConfigLocation)
            + "/jambalinux-soniccore-app.conf"
        category: "Appearance"
        property string theme: "vivo"
        property string background: "aurora"
        property int backgroundIntensity: 1
        property bool backgroundReactive: true
        // How opaque the window's backgrounds are (0.15–1), in every theme:
        // the window, the cards and the Vivo background; text and controls
        // stay solid.
        property real windowOpacity: 1
        // The radio: the style picked, Brazil only ("BR") or the world (""),
        // the favorite stations (JSON) and the last station played.
        property string radioStyle: "synthwave"
        property string radioRegion: ""
        property string radioFavorites: "[]"
        property string radioLastStation: ""
    }

    // Vivo's "Vidro" and "Papel de parede" cover the whole window with a
    // picture of their own, faded by the opacity: the window itself stays
    // clear, or the two would add up and never get see-through.
    readonly property bool pictureBackground: settings.theme === "vivo"
        && (settings.background === "glass" || settings.background === "wallpaper")
    // Premultiplied by the opacity: Qt clears the window with this color as
    // given, and the compositor reads the window as premultiplied, so a
    // plain color would be added in full over what is behind (over a dark
    // desktop the window looked the same at any opacity).
    readonly property real clearOpacity: pictureBackground ? 0 : settings.windowOpacity
    color: Qt.rgba(Kirigami.Theme.backgroundColor.r * clearOpacity,
        Kirigami.Theme.backgroundColor.g * clearOpacity,
        Kirigami.Theme.backgroundColor.b * clearOpacity, clearOpacity)

    // "Vidro" is frosted glass: below 100% the desktop compositor blurs
    // what is behind the window (KWin; elsewhere the glass keeps its own
    // blurred wallpaper).
    readonly property bool wantsBlur: visible && settings.theme === "vivo"
        && settings.background === "glass" && settings.windowOpacity < 0.999
    property bool blurred: false
    function updateBlur() {
        if (wantsBlur || blurred)
            blurred = Sonic.App.setBlurBehind(window, wantsBlur)
    }
    onWantsBlurChanged: Qt.callLater(updateBlur)


    // The window's size as the user left it, kept across closing, logging
    // out and rebooting. Wayland does not let an app choose its position;
    // KWin places the window.
    Settings {
        id: windowState
        location: settings.location
        category: "Window"
        property int width: 0
        property int height: 0
        property bool maximized: false
    }

    Component.onCompleted: {
        // Started for the tray on a desktop without one: there is nothing to
        // show at login, and the menu entry opens the window when wanted.
        if (startedInTray && !Sonic.App.trayAvailable) {
            Qt.callLater(Qt.quit)
            return
        }
        // "Transparente" was a background before the opacity control existed.
        if (settings.background === "transparent")
            settings.background = "none"
        if (windowState.width >= minimumWidth && windowState.height >= minimumHeight) {
            width = windowState.width
            height = windowState.height
        }
        if (visible && windowState.maximized)
            showMaximized()
        Qt.callLater(updateBlur)
    }

    // Remember the normal (not maximized) size shortly after a resize stops,
    // written to disk at once: a shutdown may never close the window.
    function rememberSize() {
        if (visibility === Window.Windowed) {
            windowState.width = width
            windowState.height = height
        }
        if (visibility === Window.Windowed || visibility === Window.Maximized)
            windowState.maximized = visibility === Window.Maximized
        windowState.sync()
    }

    Timer {
        id: rememberSizeTimer
        interval: 600
        onTriggered: window.rememberSize()
    }

    onWidthChanged: rememberSizeTimer.restart()
    onHeightChanged: rememberSizeTimer.restart()
    onVisibilityChanged: rememberSizeTimer.restart()

    SonicCore {
        id: core
        configuration: settings
        // Animations and polling run only while the window is on screen.
        expanded: window.visible && window.visibility !== Window.Minimized
        onToggleRequested: window.toggle()
    }

    function openSettings(page, title) {
        if (pageStack.layers.depth > 1)
            pageStack.layers.pop()
        pageStack.layers.push(page, { "title": title })
    }

    function toggle() {
        if (window.visible && window.active) {
            window.hide()
        } else {
            if (windowState.maximized)
                window.showMaximized()
            else
                window.show()
            window.raise()
            window.requestActivate()
        }
    }

    // A second launch (the menu entry while it runs in the tray).
    Connections {
        target: Sonic.App
        function onActivateRequested() {
            if (windowState.maximized)
                window.showMaximized()
            else
                window.show()
            window.raise()
            window.requestActivate()
        }
    }

    onClosing: function(close) {
        window.rememberSize()
        if (tray.available && Sonic.App.trayAvailable) {
            close.accepted = false
            window.hide()
        }
    }

    readonly property string batteryText: (core.headsetConnected !== false && core.batteryPercent >= 0
        ? `${core.batteryPercent}%` : "fone desligado")
        + (core.charging ? " (carregando)" : "")

    // The widget's panel battery, pre-drawn per 5% step by
    // tools/build-tray-icons.sh (a tray icon is a picture, not QML).
    readonly property url trayIcon: {
        // Off (or no reading) still shows the bolt while on the USB cable.
        if (core.headsetConnected === false || core.batteryPercent < 0)
            return Qt.resolvedUrl(`icons/battery-unknown${core.charging ? "-charging" : ""}.svg`)
        const step = Math.min(100, Math.max(0, Math.round(core.batteryPercent / 5) * 5))
        return Qt.resolvedUrl(`icons/battery-${step}${core.charging ? "-charging" : ""}.svg`)
    }

    Platform.SystemTrayIcon {
        id: tray
        visible: true
        icon.source: window.trayIcon
        tooltip: `JamBaLinux Sonic Core — ${window.batteryText}`
        onActivated: function(reason) {
            if (reason === Platform.SystemTrayIcon.Trigger) window.toggle()
        }
        menu: Platform.Menu {
            Platform.MenuItem {
                text: window.batteryText
                enabled: false
            }
            Platform.MenuItem {
                text: window.visible ? "Esconder" : "Abrir"
                onTriggered: window.toggle()
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                text: "Sair"
                onTriggered: {
                    window.rememberSize()
                    Qt.quit()
                }
            }
        }
    }

    pageStack.initialPage: Kirigami.ScrollablePage {
        id: page
        title: "Sonic Core"
        // Let the window colour (and its transparency) show through.
        background: Rectangle { color: "transparent" }
        padding: 0
        leftPadding: 0
        rightPadding: 0
        topPadding: 0
        bottomPadding: 0

        actions: [
            Kirigami.Action {
                text: settings.theme === "hifi" ? "Hi-Fi" : settings.theme === "vivo" ? "Vivo" : "Leve"
                icon.name: "color-management"
                tooltip: "Trocar o tema"
                Kirigami.Action {
                    text: "Vivo"
                    checkable: true
                    checked: settings.theme === "vivo"
                    onTriggered: settings.theme = "vivo"
                }
                Kirigami.Action {
                    text: "Hi-Fi"
                    tooltip: "Um rack de som dos anos 80: madeira, alumínio escovado e display verde"
                    checkable: true
                    checked: settings.theme === "hifi"
                    onTriggered: settings.theme = "hifi"
                }
                Kirigami.Action {
                    text: "Leve"
                    checkable: true
                    checked: settings.theme !== "vivo" && settings.theme !== "hifi"
                    onTriggered: settings.theme = "leve"
                }
            },
            Kirigami.Action {
                text: "Fundo"
                icon.name: "preferences-desktop-wallpaper"
                visible: settings.theme === "vivo"
                Kirigami.Action {
                    text: "Aurora"
                    checkable: true
                    checked: settings.background === "aurora"
                    onTriggered: settings.background = "aurora"
                }
                Kirigami.Action {
                    text: "Vidro"
                    checkable: true
                    checked: settings.background === "glass"
                    onTriggered: settings.background = "glass"
                }
                Kirigami.Action {
                    text: "Papel de parede"
                    tooltip: "O papel de parede do monitor principal"
                    checkable: true
                    checked: settings.background === "wallpaper"
                    onTriggered: settings.background = "wallpaper"
                }

                Kirigami.Action {
                    text: "Partículas"
                    checkable: true
                    checked: settings.background === "particles"
                    onTriggered: settings.background = "particles"
                }
                Kirigami.Action {
                    text: "Ondas"
                    checkable: true
                    checked: settings.background === "waves"
                    onTriggered: settings.background = "waves"
                }
                Kirigami.Action {
                    text: "Nenhum"
                    checkable: true
                    checked: settings.background === "none"
                    onTriggered: settings.background = "none"
                }
                Kirigami.Action {
                    separator: true
                }
                Kirigami.Action {
                    text: "Reagir ao som"
                    tooltip: "O fundo pulsa com os graves"
                    checkable: true
                    checked: settings.backgroundReactive
                    onTriggered: settings.backgroundReactive = !settings.backgroundReactive
                }
                Kirigami.Action {
                    text: "Intensidade"
                    Kirigami.Action {
                        text: "Suave"
                        checkable: true
                        checked: settings.backgroundIntensity === 0
                        onTriggered: settings.backgroundIntensity = 0
                    }
                    Kirigami.Action {
                        text: "Média"
                        checkable: true
                        checked: settings.backgroundIntensity === 1
                        onTriggered: settings.backgroundIntensity = 1
                    }
                    Kirigami.Action {
                        text: "Intensa"
                        checkable: true
                        checked: settings.backgroundIntensity === 2
                        onTriggered: settings.backgroundIntensity = 2
                    }
                }
            },
            // The battery economy and the audio output, as full pages over
            // the panel (Esc or the back arrow returns).
            Kirigami.Action {
                text: "Configurações"
                icon.name: "configure"
                Kirigami.Action {
                    text: "Bateria"
                    icon.name: "battery-good"
                    onTriggered: window.openSettings(core.batterySettingsPage, "Bateria")
                }
                Kirigami.Action {
                    text: "Saída de áudio"
                    icon.name: "audio-card"
                    onTriggered: window.openSettings(core.outputSettingsPage, "Saída de áudio")
                }
            },
            // Opacity of the window's backgrounds, always in the toolbar.
            Kirigami.Action {
                text: "Opacidade"
                displayComponent: RowLayout {
                    spacing: Kirigami.Units.smallSpacing
                    QQC2.Label { text: "Opacidade" }
                    QQC2.Slider {
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 8
                        from: 0.15
                        to: 1
                        stepSize: 0.05
                        value: settings.windowOpacity
                        onMoved: settings.windowOpacity = value
                    }
                    QQC2.Label {
                        text: `${Math.round(settings.windowOpacity * 100)}%`
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 2.5
                    }
                }
            }
        ]

        // The widget's popup, sized by the page instead of by Plasma.
        Item {
            implicitWidth: page.width
            implicitHeight: panel.item ? Math.max(panel.item.Layout.preferredHeight, page.height) : 0

            Loader {
                id: panel
                anchors.fill: parent
                sourceComponent: core.fullRepresentation
            }
        }
    }
}
