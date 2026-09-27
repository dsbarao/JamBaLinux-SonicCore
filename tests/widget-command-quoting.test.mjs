import assert from "node:assert/strict"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { existsSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import test from "node:test"

const widgetPath = new URL(
    "../packaging/plasma/org.jambalinux.soniccore/contents/ui/main.qml", import.meta.url)
const widgetSource = await readFile(widgetPath, "utf8")

test("widget commands use one non-login shell command builder", () => {
    assert.doesNotMatch(widgetSource, /sh -lc/)
    assert.match(widgetSource,
        /readonly property string soniccoreBinary: "\$HOME\/\.cargo\/bin\/soniccore"/)
    assert.match(widgetSource,
        /function soniccoreCommand\(cliArguments\) \{[\s\S]*?cliArguments\.map\(shellQuote\)\.join\(" "\)[\s\S]*?soniccoreBinary[\s\S]*?"\/bin\/sh -c " \+ shellQuote\(program\)/)
})

test("control arguments are quoted by the shared command builder", () => {
    const match = widgetSource.match(
        /    function runControl\(feature, value\) \{([\s\S]*?)^    \}/m)
    assert.notEqual(match, null)
    assert.match(match[1], /soniccoreCommand\(\["set", feature, value\]\)/)
    assert.doesNotMatch(match[1], /'\$\{value\}'/)
})

test("shared builder passes control metacharacters literally through both shell layers", async () => {
    const shellQuote = widgetSource.match(
        /    function shellQuote\(value\) \{([\s\S]*?)^    \}\n\n    function soniccoreCommand/m)
    const soniccoreCommand = widgetSource.match(
        /    function soniccoreCommand\(cliArguments\) \{([\s\S]*?)^    \}\n\n    function equalizerCliCommand/m)
    assert.notEqual(shellQuote, null)
    assert.notEqual(soniccoreCommand, null)

    const commandFor = new Function(`
        "use strict";
        const soniccoreBinary = "$HOME/.cargo/bin/soniccore";
        function shellQuote(value) {${shellQuote[1]}}
        function soniccoreCommand(cliArguments) {${soniccoreCommand[1]}}
        return soniccoreCommand;`)()
    const tempHome = await mkdtemp(join(tmpdir(), "soniccore-widget-command-"))
    const soniccoreDir = join(tempHome, ".cargo", "bin")
    const trapDir = join(tempHome, "trap")
    const sideEffectPath = join(tempHome, "unexpected-ran")

    try {
        await mkdir(soniccoreDir, { recursive: true })
        await writeFile(join(soniccoreDir, "soniccore"), "#!/bin/sh\nprintf '%s\\n' \"$@\"\n")
        await chmod(join(soniccoreDir, "soniccore"), 0o755)
        await mkdir(trapDir)
        await writeFile(join(trapDir, "unexpected"), "#!/bin/sh\n: > \"$SIDE_EFFECT_FILE\"\n")
        await chmod(join(trapDir, "unexpected"), 0o755)

        const command = commandFor([
            "set",
            "ambient; unexpected",
            "O'Reilly $(unexpected)"
        ])
        const result = spawnSync("/bin/sh", ["-c", command], {
            encoding: "utf8",
            env: {
                ...process.env,
                HOME: tempHome,
                PATH: `${trapDir}:${process.env.PATH ?? ""}`,
                SIDE_EFFECT_FILE: sideEffectPath
            }
        })

        assert.equal(result.error, undefined)
        assert.equal(result.status, 0, result.stderr)
        assert.equal(result.stdout, "set\nambient; unexpected\nO'Reilly $(unexpected)\n")
        assert.equal(existsSync(sideEffectPath), false, "unexpected command must not run")
    } finally {
        await rm(tempHome, { recursive: true, force: true })
    }
})

test("stable command properties remain the DataSource identities", () => {
    for (const name of ["command", "cachedCommand", "equalizerCommand", "spatialCommand"]) {
        assert.match(widgetSource, new RegExp(`readonly property string ${name}: soniccoreCommand\\(`))
    }
    assert.match(widgetSource, /sourceName !== root\.command/)
    assert.match(widgetSource, /sourceName === root\.equalizerCommand/)
})
