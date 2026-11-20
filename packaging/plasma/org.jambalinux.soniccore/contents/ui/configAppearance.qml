import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    property string cfg_theme
    property string cfg_themeDefault
    property string cfg_background
    property string cfg_backgroundDefault
    property int cfg_backgroundIntensity
    property int cfg_backgroundIntensityDefault
    property bool cfg_backgroundReactive
    property bool cfg_backgroundReactiveDefault

    readonly property var backgroundValues: ["aurora", "glass", "none"]

    Kirigami.FormLayout {
        QQC2.RadioButton {
            id: lightTheme
            Kirigami.FormData.label: "Tema:"
            text: "Leve"
            checked: cfg_theme !== "vivo"
            onToggled: if (checked) cfg_theme = "leve"
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            text: "A interface simples, sem animações. Não usa o medidor de espectro."
            wrapMode: Text.Wrap
            opacity: 0.7
        }
        QQC2.RadioButton {
            text: "Vivo"
            checked: cfg_theme === "vivo"
            onToggled: if (checked) cfg_theme = "vivo"
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            text: "Espectro ao vivo, caminho do som animado e brilho nas cores das luzes do fone. "
                + "Só anima com o painel aberto e respeita a opção de reduzir animações do sistema."
            wrapMode: Text.Wrap
            opacity: 0.7
        }

        Item { Kirigami.FormData.isSection: true }

        QQC2.ComboBox {
            Kirigami.FormData.label: "Fundo do tema Vivo:"
            enabled: cfg_theme === "vivo"
            model: ["Aurora (luzes nas cores do fone)", "Vidro (papel de parede desfocado)", "Nenhum"]
            currentIndex: Math.max(0, backgroundValues.indexOf(cfg_background))
            onActivated: cfg_background = backgroundValues[currentIndex]
        }
        QQC2.ComboBox {
            Kirigami.FormData.label: "Intensidade:"
            enabled: cfg_theme === "vivo" && cfg_background !== "none"
            model: ["Sutil", "Média", "Intensa"]
            currentIndex: Math.max(0, Math.min(2, cfg_backgroundIntensity))
            onActivated: cfg_backgroundIntensity = currentIndex
        }
        QQC2.CheckBox {
            enabled: cfg_theme === "vivo" && cfg_background !== "none"
            text: "Reagir ao som (o grave faz o fundo pulsar)"
            checked: cfg_backgroundReactive
            onToggled: cfg_backgroundReactive = checked
        }
    }
}
