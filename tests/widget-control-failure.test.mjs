import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/app/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

test("Uma falha de comando de controle aparece uma única vez, na mensagem do topo, em cor negativa", () => {
    const onNewDataMatch = widgetSource.match(/if \(sourceName !== root\.command\) \{([\s\S]+?)return\s*\n\s*\}/)
    assert.ok(onNewDataMatch, "Não encontrou if (sourceName !== root.command)")
    
    const controlHandler = onNewDataMatch[1]
    
    const elseBlockMatch = controlHandler.match(/\} else \{([\s\S]+?)\}/)
    assert.ok(elseBlockMatch, "Não encontrou bloco else na falha do controle")
    const elseBlock = elseBlockMatch[1]

    assert.match(elseBlock, /root\.actionFailed\s*=\s*true/, "Deve setar actionFailed = true")
    assert.match(elseBlock, /root\.actionMessage\s*=\s*root\.friendlyError\(stderr,\s*exitCode,\s*"write"\)/, "Deve setar actionMessage com o erro")
    assert.doesNotMatch(elseBlock, /root\.errorMessage\s*=/, "Não deve copiar a falha de controle para errorMessage")
})

test("Depois de uma falha de comando de controle, clearAction e refreshAfterControl são reiniciados com temporização", () => {
    const onNewDataMatch = widgetSource.match(/if \(sourceName !== root\.command\) \{([\s\S]+?)return\s*\n\s*\}/)
    const elseBlock = onNewDataMatch[1].match(/\} else \{([\s\S]+?)\}/)[1]

    assert.match(elseBlock, /clearAction\.interval\s*=\s*\d+/, "Deve ajustar o interval do clearAction")
    assert.match(elseBlock, /clearAction\.restart\(\)/, "Deve reiniciar clearAction")
    assert.match(elseBlock, /refreshAfterControl\.restart\(\)/, "Deve reiniciar refreshAfterControl")
})

test("A cor da mensagem do topo depende só do resultado da ação", () => {
    const actionLabelMatch = widgetSource.match(/text:\s*root\.actionMessage\s*\n[\s\S]*?color:\s*([\s\S]+?)wrapMode:/m)
    assert.ok(actionLabelMatch, "Não encontrou color no Label de actionMessage")
    
    const colorExpr = actionLabelMatch[1]
    assert.match(colorExpr, /root\.actionFailed\s*\?/, "A cor deve depender de root.actionFailed")
    assert.doesNotMatch(colorExpr, /errorMessage/, "A cor não deve depender de errorMessage")
})

test("Caminhos de sucesso resetam actionFailed", () => {
    assert.match(widgetSource, /actionFailed\s*=\s*false[\s\S]{0,100}actionMessage\s*=\s*"Aplicando…"/, "runControl deve deixar actionFailed falso")
    assert.match(widgetSource, /root\.actionFailed\s*=\s*false[\s\S]{0,100}root\.actionMessage\s*=\s*"Configuração aplicada"/, "Sucesso do controle deve deixar actionFailed falso")
    assert.match(widgetSource, /root\.actionFailed\s*=\s*false[\s\S]{0,100}root\.actionMessage\s*=\s*root\.equalizerActionMessage/, "Sucesso do equalizador deve deixar actionFailed falso")
    
    const clearActionMatch = widgetSource.match(/id:\s*clearAction[\s\S]+?onTriggered:\s*\{([\s\S]+?)\}/)
    assert.ok(clearActionMatch, "clearAction deve ter onTriggered em bloco")
    assert.match(clearActionMatch[1], /root\.actionFailed\s*=\s*false/, "clearAction deve resetar actionFailed para falso")
})
