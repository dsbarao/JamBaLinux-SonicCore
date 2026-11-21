import QtCore
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import Qt.labs.platform as Platform

import org.kde.kirigami as Kirigami

import "../plasma/org.jambalinux.soniccore/contents/ui"

// SonicCore as a standalone app: the same interface as the Plasma widget
// (SonicCore.qml) in its own window, with a tray icon showing the battery.
// Closing the window keeps it in the tray; "Sair" quits.
//
// Installed by tools/install-user.sh and started by soniccore-app; from the
// repository: `qml6 --apptype widget packaging/app/main.qml [-- --tray]`.
Kirigami.ApplicationWindow {
    id: window

    title: "JamBaLinux SonicCore"
    width: Kirigami.Units.gridUnit * 68
    height: Kirigami.Units.gridUnit * 46
    minimumWidth: Kirigami.Units.gridUnit * 25
    minimumHeight: Kirigami.Units.gridUnit * 24
    // At login (autostart passes --tray) only the tray icon appears.
    visible: !Qt.application.arguments.includes("--tray")

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
    }

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
        if (windowState.width >= minimumWidth && windowState.height >= minimumHeight) {
            width = windowState.width
            height = windowState.height
        }
        if (visible && windowState.maximized)
            showMaximized()
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

    onClosing: function(close) {
        window.rememberSize()
        if (tray.available) {
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
        tooltip: `JamBaLinux SonicCore — ${window.batteryText}`
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
        title: "SonicCore"
        padding: 0
        leftPadding: 0
        rightPadding: 0
        topPadding: 0
        bottomPadding: 0

        actions: [
            Kirigami.Action {
                text: settings.theme === "vivo" ? "Vivo" : "Leve"
                icon.name: "color-management"
                tooltip: "Trocar o tema"
                onTriggered: settings.theme = settings.theme === "vivo" ? "leve" : "vivo"
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
