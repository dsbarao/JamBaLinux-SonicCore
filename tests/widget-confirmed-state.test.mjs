import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/SonicCore.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

test("confirmed-state buttons do not use checkable", () => {
    // Ambient buttons
    assert.doesNotMatch(widgetSource, /text:\s*"Desligado"[^\n]*?checkable:\s*true[^\n]*?root\.ambientMode/)
    assert.doesNotMatch(widgetSource, /text:\s*"ANC"[^\n]*?checkable:\s*true[^\n]*?root\.ambientMode/)
    assert.doesNotMatch(widgetSource, /text:\s*"TalkThru"[^\n]*?checkable:\s*true[^\n]*?root\.ambientMode/)

    // Sidetone buttons
    assert.doesNotMatch(widgetSource, /text:\s*"Off"[^\n]*?checkable:\s*true[^\n]*?root\.sidetoneLevel/)
    assert.doesNotMatch(widgetSource, /text:\s*"Baixo"[^\n]*?checkable:\s*true[^\n]*?root\.sidetoneLevel/)
    assert.doesNotMatch(widgetSource, /text:\s*"Médio"[^\n]*?checkable:\s*true[^\n]*?root\.sidetoneLevel/)
    assert.doesNotMatch(widgetSource, /text:\s*"Alto"[^\n]*?checkable:\s*true[^\n]*?root\.sidetoneLevel/)

    // Lighting On/Off buttons
    assert.doesNotMatch(widgetSource, /text:\s*"Ligar"[^\n]*?checkable:\s*true[^\n]*?root\.lightingEnabled/)
    assert.doesNotMatch(widgetSource, /text:\s*"Desligar"[^\n]*?checkable:\s*true[^\n]*?root\.lightingEnabled/)

    // Lighting effect buttons
    assert.doesNotMatch(widgetSource, /text:\s*"Respiração"[^\n]*?checkable:\s*true[^\n]*?selectedLightingEffect/)
    assert.doesNotMatch(widgetSource, /text:\s*"Sólido"[^\n]*?checkable:\s*true[^\n]*?selectedLightingEffect/)
    assert.doesNotMatch(widgetSource, /text:\s*"Onda"[^\n]*?checkable:\s*true[^\n]*?selectedLightingEffect/)
    assert.doesNotMatch(widgetSource, /text:\s*"Falha"[^\n]*?checkable:\s*true[^\n]*?selectedLightingEffect/)

    // Lighting speed buttons
    assert.doesNotMatch(widgetSource, /text:\s*"0,5×"[^\n]*?checkable:\s*true[^\n]*?selectedLightingSpeed/)
    assert.doesNotMatch(widgetSource, /text:\s*"1×"[^\n]*?checkable:\s*true[^\n]*?selectedLightingSpeed/)
    assert.doesNotMatch(widgetSource, /text:\s*"1,5×"[^\n]*?checkable:\s*true[^\n]*?selectedLightingSpeed/)
    assert.doesNotMatch(widgetSource, /text:\s*"2×"[^\n]*?checkable:\s*true[^\n]*?selectedLightingSpeed/)
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

test("listening modes: Natural turns the 3D off, a named mode sets its levels and turns it on", () => {
    assert.match(widgetSource, /\{ "mode": "natural", "label": "Natural" \}/)
    assert.match(widgetSource, /\{ "mode": "custom", "label": "Personalizado" \}/)
    assert.match(widgetSource, /checked: modelData\.mode === "natural"\n\s+\? !root\.spatialEnabled\n\s+: root\.spatialEnabled && root\.spatialListening === modelData\.mode/)
    assert.match(widgetSource, /if \(spatialEnabled\) setSpatialEnabled\(false\)/)
    assert.match(widgetSource, /const steps = \[\["spatial", "sound", mode\]\]\n\s+if \(!spatialEnabled\) steps\.push\(\["spatial", "enable"\]\)/)
    assert.match(widgetSource, /\["spatial", "sound", "set", control,/)
    assert.doesNotMatch(widgetSource, /experimental|Recurso experimental|saudável/)
})

test("equalizer switch shows the confirmed bypass state and folds the bands away", () => {
    const switchMatch = widgetSource.match(/PlasmaComponents\.Switch\s*\{(\s*id:\s*equalizerToggle[\s\S]*?)\n\s*\}/)
    assert.notEqual(switchMatch, null, "the equalizer card must have its on/off switch")
    assert.match(switchMatch[1], /checked:\s*!root\.equalizerBypassed/)
    assert.match(switchMatch[1], /checked\s*=\s*Qt\.binding\(function\(\)\s*\{\s*return\s+!root\.equalizerBypassed\s*\}\)/)
    assert.match(switchMatch[1], /root\.setEqualizerBypassed\(!on\)/)
    assert.match(widgetSource, /startEqualizerAction\(bypassed \? "bypass-on" : "bypass-off", \["bypass", bypassed \? "on" : "off"\]\)/)
    assert.match(widgetSource, /visible: !root\.equalizerBypassed/)
    assert.match(widgetSource, /root\.equalizerBypassed = profile\.bypassed === true/)
})
