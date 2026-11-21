import QtQuick
import org.kde.plasma.configuration

ConfigModel {
    ConfigCategory {
        name: "Aparência"
        icon: "preferences-desktop-theme"
        source: "configAppearance.qml"
    }
    ConfigCategory {
        name: "Saída de som"
        icon: "audio-volume-high"
        source: "configOutput.qml"
    }
    ConfigCategory {
        name: "Bateria"
        icon: "battery-good"
        source: "configBattery.qml"
    }
}
