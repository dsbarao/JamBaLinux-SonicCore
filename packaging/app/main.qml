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
// Sketch: run with `qml6 --apptype widget packaging/app/main.qml`.
Kirigami.ApplicationWindow {
    id: window

    title: "JamBaLinux SonicCore"
    width: Kirigami.Units.gridUnit * 28
    height: Kirigami.Units.gridUnit * 44
    minimumWidth: Kirigami.Units.gridUnit * 25
    minimumHeight: Kirigami.Units.gridUnit * 24
    visible: true

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
            window.show()
            window.raise()
            window.requestActivate()
        }
    }

    onClosing: function(close) {
        if (tray.available) {
            close.accepted = false
            window.hide()
        }
    }

    readonly property string batteryText: core.batteryPercent >= 0
        ? `${core.batteryPercent}%${core.charging ? " (carregando)" : ""}`
        : "fone desligado"

    Platform.SystemTrayIcon {
        id: tray
        visible: true
        icon.name: "audio-headphones"
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
                onTriggered: Qt.quit()
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
