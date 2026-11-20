import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    property string cfg_theme
    property string cfg_themeDefault

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
    }
}
