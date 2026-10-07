import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/main.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

test("confirmed-state buttons do not use checkable", () => {
    // Ambient buttons
    assert.doesNotMatch(widgetSource, /text:\s*"Desligado"[\s\S]*?checkable:\s*true[\s\S]*?root\.ambientMode/)
    assert.doesNotMatch(widgetSource, /text:\s*"ANC"[\s\S]*?checkable:\s*true[\s\S]*?root\.ambientMode/)
    assert.doesNotMatch(widgetSource, /text:\s*"TalkThru"[\s\S]*?checkable:\s*true[\s\S]*?root\.ambientMode/)

    // Sidetone buttons
    assert.doesNotMatch(widgetSource, /text:\s*"Off"[\s\S]*?checkable:\s*true[\s\S]*?root\.sidetoneLevel/)
    assert.doesNotMatch(widgetSource, /text:\s*"Baixo"[\s\S]*?checkable:\s*true[\s\S]*?root\.sidetoneLevel/)
    assert.doesNotMatch(widgetSource, /text:\s*"Médio"[\s\S]*?checkable:\s*true[\s\S]*?root\.sidetoneLevel/)
    assert.doesNotMatch(widgetSource, /text:\s*"Alto"[\s\S]*?checkable:\s*true[\s\S]*?root\.sidetoneLevel/)

    // Lighting On/Off buttons
    assert.doesNotMatch(widgetSource, /text:\s*"Ligar"[\s\S]*?checkable:\s*true[\s\S]*?root\.lightingEnabled/)
    assert.doesNotMatch(widgetSource, /text:\s*"Desligar"[\s\S]*?checkable:\s*true[\s\S]*?root\.lightingEnabled/)

    // Lighting effect buttons
    assert.doesNotMatch(widgetSource, /text:\s*"Respiração"[\s\S]*?checkable:\s*true[\s\S]*?selectedLightingEffect/)
    assert.doesNotMatch(widgetSource, /text:\s*"Sólido"[\s\S]*?checkable:\s*true[\s\S]*?selectedLightingEffect/)
    assert.doesNotMatch(widgetSource, /text:\s*"Onda"[\s\S]*?checkable:\s*true[\s\S]*?selectedLightingEffect/)
    assert.doesNotMatch(widgetSource, /text:\s*"Falha"[\s\S]*?checkable:\s*true[\s\S]*?selectedLightingEffect/)

    // Lighting speed buttons
    assert.doesNotMatch(widgetSource, /text:\s*"0,5×"[\s\S]*?checkable:\s*true[\s\S]*?selectedLightingSpeed/)
    assert.doesNotMatch(widgetSource, /text:\s*"1×"[\s\S]*?checkable:\s*true[\s\S]*?selectedLightingSpeed/)
    assert.doesNotMatch(widgetSource, /text:\s*"1,5×"[\s\S]*?checkable:\s*true[\s\S]*?selectedLightingSpeed/)
    assert.doesNotMatch(widgetSource, /text:\s*"2×"[\s\S]*?checkable:\s*true[\s\S]*?selectedLightingSpeed/)
})

test("confirmed-state buttons keep checked bound to root state", () => {
    // Ambient
    assert.match(widgetSource, /text:\s*"Desligado"[^}]*checked:\s*root\.ambientMode === "off"/)
    assert.match(widgetSource, /text:\s*"ANC"[^}]*checked:\s*root\.ambientMode === "anc"/)
    assert.match(widgetSource, /text:\s*"TalkThru"[^}]*checked:\s*root\.ambientMode === "talkthru"/)

    // Sidetone
    assert.match(widgetSource, /text:\s*"Off"[^}]*checked:\s*root\.sidetoneLevel === "off"/)
    assert.match(widgetSource, /text:\s*"Baixo"[^}]*checked:\s*root\.sidetoneLevel === "low"/)
    assert.match(widgetSource, /text:\s*"Médio"[^}]*checked:\s*root\.sidetoneLevel === "medium"/)
    assert.match(widgetSource, /text:\s*"Alto"[^}]*checked:\s*root\.sidetoneLevel === "high"/)

    // Lighting On/Off
    assert.match(widgetSource, /text:\s*"Ligar"[^}]*checked:\s*root\.lightingEnabled === true/)
    assert.match(widgetSource, /text:\s*"Desligar"[^}]*checked:\s*root\.lightingEnabled === false/)

    // Effect
    assert.match(widgetSource, /text:\s*"Respiração"[^}]*checked:\s*root\.selectedLightingEffect\(\) === "breathing"/)
    assert.match(widgetSource, /text:\s*"Sólido"[^}]*checked:\s*root\.selectedLightingEffect\(\) === "solid"/)
    assert.match(widgetSource, /text:\s*"Onda"[^}]*checked:\s*root\.selectedLightingEffect\(\) === "wave"/)
    assert.match(widgetSource, /text:\s*"Falha"[^}]*checked:\s*root\.selectedLightingEffect\(\) === "glitch"/)

    // Speed
    assert.match(widgetSource, /text:\s*"0,5×"[^}]*checked:\s*root\.selectedLightingSpeed\(\) === "0.5"/)
    assert.match(widgetSource, /text:\s*"1×"[^}]*checked:\s*root\.selectedLightingSpeed\(\) === "1.0"/)
    assert.match(widgetSource, /text:\s*"1,5×"[^}]*checked:\s*root\.selectedLightingSpeed\(\) === "1.5"/)
    assert.match(widgetSource, /text:\s*"2×"[^}]*checked:\s*root\.selectedLightingSpeed\(\) === "2.0"/)
})

test("spatial switch reasserts checked binding on click and is not referenced outside Switch", () => {
    // Switch block has id spatialToggle and reasserts checked binding on click
    const switchMatch = widgetSource.match(/PlasmaComponents\.Switch\s*\{([\s\S]*?)\n\s*\}/)
    assert.notEqual(switchMatch, null, "PlasmaComponents.Switch must exist")
    assert.match(switchMatch[1], /id:\s*spatialToggle/)
    assert.match(switchMatch[1], /checked\s*=\s*Qt\.binding\(function\(\)\s*\{\s*return\s+root\.spatialEnabled;?\s*\}\)/)

    // spatialToggle must only be referenced inside the Switch block
    const nonSwitchSource = widgetSource.replace(switchMatch[0], "")
    assert.doesNotMatch(nonSwitchSource, /\bspatialToggle\b/)
})
