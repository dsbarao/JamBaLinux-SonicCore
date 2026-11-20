import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM
import org.kde.plasma.plasma5support as Plasma5Support

// The economy lives in the SonicCore daemon (it must work with the popup
// closed), so these choices are applied immediately through the CLI rather
// than stored in the widget's own configuration.
KCM.SimpleKCM {
    id: page

    // Same unexpanded path as main.qml; the installer rewrites both for a
    // custom CARGO_HOME.
    readonly property string soniccoreBinary: "$HOME/.cargo/bin/soniccore"
    readonly property var lowBatteryChoices: ["off", "10", "15", "20", "30"]
    readonly property var idleChoices: ["off", "5", "10", "15", "30"]
    property bool loaded: false
    property string message: ""
    property string drainText: ""

    function run(arguments) {
        // Only fixed words and numbers from the lists above reach the shell.
        runner.connectSource("/bin/sh -c '" + soniccoreBinary + " " + arguments.join(" ") + "'")
    }

    function apply(setting, value) {
        if (!loaded) return
        message = "Aplicando…"
        run(["power", "set", setting, value])
    }

    Component.onCompleted: run(["power", "--format", "json"])

    Plasma5Support.DataSource {
        id: runner
        engine: "executable"
        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            const exitCode = Number(data["exit code"] ?? -1)
            if (sourceName.indexOf(" power set ") >= 0) {
                page.message = exitCode === 0 ? "Aplicado." : "Não foi possível aplicar: " + String(data.stderr ?? "").trim()
                return
            }
            if (exitCode !== 0) {
                page.message = "Não foi possível ler a economia de bateria. O SonicCore está instalado?"
                return
            }
            try {
                const power = JSON.parse(String(data.stdout ?? ""))
                const low = power.config.low_battery_percent
                const idle = power.config.idle_minutes
                lowBattery.currentIndex = Math.max(0, page.lowBatteryChoices.indexOf(low === null ? "off" : String(low)))
                idleBox.currentIndex = Math.max(0, page.idleChoices.indexOf(idle === null ? "off" : String(idle)))
                const on = power.history.lights_on
                const off = power.history.lights_off
                page.drainText = on && off
                    ? `Consumo medido: ${on.percent_per_hour.toFixed(1)}%/h com luzes acesas, ${off.percent_per_hour.toFixed(1)}%/h com luzes apagadas.`
                    : "O consumo com e sem luzes aparece aqui depois de algumas horas de uso fora do carregador."
            } catch (error) {
                page.message = "Resposta inesperada do SonicCore."
            }
            page.loaded = true
        }
    }

    Kirigami.FormLayout {
        QQC2.ComboBox {
            id: lowBattery
            Kirigami.FormData.label: "Apagar as luzes com bateria baixa:"
            model: ["Nunca", "10% ou menos", "15% ou menos", "20% ou menos", "30% ou menos"]
            onActivated: page.apply("low-battery", page.lowBatteryChoices[currentIndex])
        }
        QQC2.ComboBox {
            id: idleBox
            Kirigami.FormData.label: "Apagar as luzes sem som por:"
            model: ["Nunca", "5 minutos", "10 minutos", "15 minutos", "30 minutos"]
            onActivated: page.apply("idle", page.idleChoices[currentIndex])
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            text: "As luzes voltam sozinhas ao carregar ou quando o som voltar, com a mesma cor e efeito. "
                + "Se você ligá-las durante a economia, elas ficam acesas. O ANC nunca é alterado."
            wrapMode: Text.Wrap
            opacity: 0.7
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            visible: page.drainText.length > 0
            text: page.drainText
            wrapMode: Text.Wrap
        }
        QQC2.Label {
            visible: page.message.length > 0
            text: page.message
            opacity: 0.8
        }
    }
}
