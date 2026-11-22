import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM
import org.kde.plasma.plasma5support as Plasma5Support

// Chooses where the equalizer and the spatial graph play. Applied at once
// through the CLI (the equalizer service is restarted for the new output);
// lighting, ANC and battery stay with the JBL Quantum 810.
KCM.SimpleKCM {
    id: page

    // Same unexpanded path as main.qml; the installer rewrites it for a
    // custom CARGO_HOME.
    readonly property string soniccoreBinary: "$HOME/.cargo/bin/soniccore"
    property var outputs: []
    property string current: ""
    property bool loaded: false
    property string message: ""

    function shellQuote(value) {
        return "'" + String(value).replace(/'/g, "'\"'\"'") + "'"
    }

    function run(arguments) {
        const program = soniccoreBinary + " " + arguments.map(shellQuote).join(" ")
        runner.connectSource("/bin/sh -c " + shellQuote(program))
    }

    function choose(index) {
        if (!loaded || index < 0 || index >= outputs.length) return
        const target = outputs[index].target
        if (target === current) return
        message = "Trocando a saída e reiniciando o equalizador…"
        run(["output", "set", target])
    }

    Component.onCompleted: run(["output", "--format", "json"])

    Plasma5Support.DataSource {
        id: runner
        engine: "executable"
        onNewData: function(sourceName, data) {
            disconnectSource(sourceName)
            const exitCode = Number(data["exit code"] ?? -1)
            if (sourceName.indexOf("output' 'set") >= 0) {
                if (exitCode === 0) {
                    page.message = "Pronto: o equalizador e o áudio espacial tocam na nova saída."
                    page.run(["output", "--format", "json"])
                } else {
                    page.message = "Não foi possível trocar: " + String(data.stderr ?? "").trim()
                }
                return
            }
            if (exitCode !== 0) {
                page.message = "Não foi possível listar as saídas. O SonicCore está instalado?"
                return
            }
            try {
                const result = JSON.parse(String(data.stdout ?? ""))
                const describe = name => {
                    const found = (result.outputs ?? []).find(output => output.name === name)
                    if (!found) return name ?? "nenhuma"
                    return found.quantum_game ? "JBL Quantum 810 — canal Game" : found.description
                }
                const list = [{
                    target: "auto",
                    label: "Automático (agora: "
                        + describe(result.effective) + ")"
                }]
                for (const output of result.outputs ?? []) {
                    list.push({
                        target: output.quantum_game ? "quantum-game" : output.name,
                        label: output.quantum_game
                            ? "JBL Quantum 810 — canal Game (padrão)"
                            : output.description
                    })
                }
                if (!list.some(item => item.target === result.target))
                    list.push({ target: result.target, label: result.target + " (desconectada)" })
                page.outputs = list
                page.current = result.target
                outputBox.currentIndex = Math.max(0, list.findIndex(item => item.target === result.target))
            } catch (error) {
                page.message = "Resposta inesperada do SonicCore."
            }
            page.loaded = true
        }
    }

    Kirigami.FormLayout {
        QQC2.ComboBox {
            id: outputBox
            Kirigami.FormData.label: "Equalizador e áudio espacial em:"
            Layout.minimumWidth: Kirigami.Units.gridUnit * 18
            Layout.preferredWidth: Kirigami.Units.gridUnit * 24
            model: page.outputs.map(item => item.label)
            onActivated: page.choose(currentIndex)
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            text: "No Automático, o equalizador e o espacial vão para a saída que você escolher no controle de volume do sistema. "
                + "O som que você tocar nessa saída passa pelo equalizador (e pelo espacial, se ligado). "
                + "Luzes, ANC, retorno e bateria continuam sendo do JBL Quantum 810. "
                + "O canal de Chat do fone nunca é usado, para manter o microfone e as chamadas isolados."
            wrapMode: Text.Wrap
            opacity: 0.7
        }
        QQC2.Label {
            visible: page.message.length > 0
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            text: page.message
            wrapMode: Text.Wrap
        }
    }
}
