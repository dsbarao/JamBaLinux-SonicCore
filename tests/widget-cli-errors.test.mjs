import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/main.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

const friendlyErrorMatch = widgetSource.match(
    /    function friendlyError\(([^)]*)\) \{([\s\S]*?)^    \}\n\n    function normalizeBatteryPercent/m)

assert.notEqual(friendlyErrorMatch, null, "main.qml must declare friendlyError before normalizeBatteryPercent")

const friendlyError = new Function(
    `function friendlyError(${friendlyErrorMatch[1]}) {${friendlyErrorMatch[2]}}\nreturn friendlyError;`)()

test("exit code 127 returns CLI not found message regardless of stderr or context", () => {
    const expected = "CLI soniccore não encontrada. Execute tools/install-user.sh."
    // bash pt-BR
    assert.equal(friendlyError("bash: soniccore: Arquivo ou diretório inexistente", 127, "read"), expected)
    assert.equal(friendlyError("bash: soniccore: Arquivo ou diretório inexistente", 127, "write"), expected)
    // dash / sh en-US
    assert.equal(friendlyError("sh: 1: soniccore: not found", 127, "read"), expected)
    assert.equal(friendlyError("sh: 1: soniccore: not found", 127, "write"), expected)
    // empty or other stderr
    assert.equal(friendlyError("", 127, "read"), expected)
    assert.equal(friendlyError("", 127, "write"), expected)
})

test("exit code 126 returns CLI permission denied message regardless of stderr or context", () => {
    const expected = "CLI soniccore sem permissão de execução. Execute tools/install-user.sh."
    assert.equal(friendlyError("bash: soniccore: Permissão negada", 126, "read"), expected)
    assert.equal(friendlyError("bash: soniccore: Permission denied", 126, "write"), expected)
    assert.equal(friendlyError("", 126, "read"), expected)
    assert.equal(friendlyError("", 126, "write"), expected)
})

test("exit code 1 with dongle not found message returns dongle disconnected", () => {
    const stderr = "JBL Quantum 810 dongle (0ecb:2069) not found"
    assert.equal(friendlyError(stderr, 1, "read"), "Dongle USB desconectado")
    assert.equal(friendlyError(stderr, 1, "write"), "Dongle USB desconectado")
    assert.equal(friendlyError("Dispositivo não encontrado", 1, "read"), "Dongle USB desconectado")
})

test("exit code 1 with permission denied message returns permission error", () => {
    assert.equal(friendlyError("permission denied opening hid device", 1, "read"), "Sem permissão para acessar o headset")
    assert.equal(friendlyError("permissão negada ao abrir dispositivo", 1, "write"), "Sem permissão para acessar o headset")
    assert.equal(friendlyError("operation not permitted", 1, "read"), "Sem permissão para acessar o headset")
})

test("generic failure distinguishes read vs write context", () => {
    assert.equal(friendlyError("unexpected error occurred", 1, "read"), "Não foi possível ler o headset")
    assert.equal(friendlyError("unexpected error occurred", 1, "write"), "Não foi possível aplicar a configuração")
    assert.equal(friendlyError("", 1, "read"), "Não foi possível ler o headset")
    assert.equal(friendlyError("", 1, "write"), "Não foi possível aplicar a configuração")
})

test("main.qml passes exit code and context at both invocation sites", () => {
    // Read status invocation site
    assert.match(widgetSource, /errorMessage\s*=\s*friendlyError\(stderr,\s*exitCode,\s*"read"\)/)
    // Control / set invocation site
    assert.match(widgetSource, /root\.errorMessage\s*=\s*root\.friendlyError\(stderr,\s*exitCode,\s*"write"\)/)
})
