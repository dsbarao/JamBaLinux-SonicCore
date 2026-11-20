// Logic tests for the OpenRGB plugin, without OpenRGB itself: command
// planning, colour conversion, the shell-free process runner, the cached
// status reader and the coalescing writer. Built and run by
// tests/openrgb_plugin_logic.rs.

#include <cassert>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>
#include <sys/stat.h>
#include <thread>
#include <vector>

#include "../openrgb-plugin/LightingPlan.h"
#include "../openrgb-plugin/SoniccoreClient.h"

using namespace jambalinux;

static int failures = 0;

#define CHECK(condition)                                                        \
    do {                                                                        \
        if(!(condition)) {                                                      \
            std::cerr << __FILE__ << ":" << __LINE__ << ": " #condition "\n";   \
            failures++;                                                         \
        }                                                                       \
    } while(0)

static std::string scratch;

static std::string WriteScript(const std::string& name, const std::string& body)
{
    std::string path = scratch + "/" + name;
    std::ofstream(path) << "#!/bin/sh\n" << body;
    chmod(path.c_str(), 0755);
    return path;
}

static std::vector<std::string> ReadLines(const std::string& path)
{
    std::vector<std::string> lines;
    std::ifstream input(path);
    for(std::string line; std::getline(input, line);)
    {
        lines.push_back(line);
    }
    return lines;
}

static LightingState Solid(uint32_t color)
{
    LightingState state;
    state.colors.fill(color);
    return state;
}

static void TestColors()
{
    // OpenRGB stores 0x00BBGGRR.
    CHECK(ColorHex(0x00332211) == "#112233");
    CHECK(ParseColorHex("#112233") == std::optional<uint32_t>(0x00332211));
    CHECK(!ParseColorHex("112233"));
    CHECK(!ParseColorHex("#1122"));
    CHECK(!ParseColorHex("#11223g"));
}

static void TestPlan()
{
    LightingState red = Solid(0x0000ff);

    // Unknown device state: colors first, then the effect.
    std::vector<Command> first = PlanCommands(std::nullopt, red);
    CHECK(first.size() == 2);
    CHECK(first[0][1] == "leds");
    CHECK(first[0][2].rfind("#ff0000,", 0) == 0);
    CHECK(first[1] == (Command{ "set", "effect", "solid" }));

    // Already applied: nothing to send.
    CHECK(PlanCommands(red, red).empty());

    // Only one LED changed: one profile write.
    LightingState one = red;
    one.colors[7] = 0x00ff00;
    std::vector<Command> led = PlanCommands(red, one);
    CHECK(led.size() == 1 && led[0][1] == "leds");

    // Animated effect sends the speed too; a speed change alone sends only it.
    LightingState wave = red;
    wave.mode = MODE_WAVE;
    wave.speed = 2;
    std::vector<Command> to_wave = PlanCommands(red, wave);
    CHECK(to_wave.size() == 3);
    CHECK(to_wave[1] == (Command{ "set", "effect", "wave" }));
    CHECK(to_wave[2] == (Command{ "set", "speed", "1.5" }));
    LightingState faster = wave;
    faster.speed = 9;   // clamped
    CHECK(PlanCommands(wave, faster) == (std::vector<Command>{ { "set", "speed", "2.0" } }));

    // Off is one command, and is not repeated.
    LightingState off = red;
    off.mode = MODE_OFF;
    CHECK(PlanCommands(red, off) == (std::vector<Command>{ { "set", "lighting", "off" } }));
    CHECK(PlanCommands(off, off).empty());
    // Leaving Off rewrites the whole profile, which turns lighting back on.
    CHECK(PlanCommands(off, red).size() == 2);

    // Invalid mode: nothing.
    LightingState bad = red;
    bad.mode = 42;
    CHECK(PlanCommands(red, bad).empty());
}

static void TestRunCommand()
{
    std::string args_file = scratch + "/args.txt";
    std::string echo = WriteScript("echo-args",
        "for argument in \"$@\"; do printf '%s\\n' \"$argument\"; done > '" + args_file + "'\n"
        "echo out; echo err >&2; exit 3\n");

    CommandResult result = RunCommand(echo, { "set", "leds", "a b;$(touch x)", "'q'" },
                                      std::chrono::milliseconds(5000));
    CHECK(result.exit_code == 3);
    CHECK(result.out == "out\n");
    CHECK(result.err == "err\n");
    // Arguments arrive verbatim: no shell ever parses them.
    CHECK(ReadLines(args_file) == (std::vector<std::string>{ "set", "leds", "a b;$(touch x)", "'q'" }));
    struct stat info;
    CHECK(stat("x", &info) != 0);

    std::string slow = WriteScript("slow", "exec sleep 10\n");
    auto started = std::chrono::steady_clock::now();
    CommandResult timed_out = RunCommand(slow, {}, std::chrono::milliseconds(200));
    CHECK(timed_out.exit_code == -1);
    CHECK(std::chrono::steady_clock::now() - started < std::chrono::seconds(3));

    CHECK(RunCommand("", {}, std::chrono::milliseconds(100)).exit_code == -1);
    CHECK(RunCommand(scratch + "/missing", {}, std::chrono::milliseconds(100)).exit_code == -1);
}

static void TestCachedLighting()
{
    std::string status = WriteScript("status-ok",
        "cat <<'EOF'\n"
        "{\"lighting_enabled\": true, \"logo_effect\": \"wave\", \"logo_speed\": \"1.5\","
        " \"logo_colors\": [\"#112233\",\"#112233\",\"#112233\",\"#112233\",\"#ff0000\"],"
        " \"ring_colors\": [\"#00ff00\",\"#00ff00\",\"#00ff00\",\"#00ff00\",\"#0000ff\"]}\n"
        "EOF\n");
    std::optional<LightingState> state = ReadCachedLighting(status);
    CHECK(state.has_value());
    if(state)
    {
        CHECK(state->mode == MODE_WAVE);
        CHECK(state->speed == 2);
        CHECK(state->colors[0] == 0x00332211);
        CHECK(state->colors[4] == 0x000000ff);
        CHECK(state->colors[9] == 0x00ff0000);
    }

    std::string off = WriteScript("status-off",
        "echo '{\"lighting_enabled\": false, \"logo_effect\": \"solid\", \"logo_speed\": \"0.5\","
        " \"logo_colors\": [\"#000000\",\"#000000\",\"#000000\",\"#000000\",\"#000000\"],"
        " \"ring_colors\": [\"#000000\",\"#000000\",\"#000000\",\"#000000\",\"#000000\"]}'\n");
    std::optional<LightingState> off_state = ReadCachedLighting(off);
    CHECK(off_state && off_state->mode == MODE_OFF);

    // Unknown zone colors (after a partial write) or a failed read: no state.
    CHECK(!ReadCachedLighting(WriteScript("status-unknown",
        "echo '{\"lighting_enabled\": true, \"logo_effect\": \"solid\", \"logo_colors\": null,"
        " \"ring_colors\": null}'\n")));
    CHECK(!ReadCachedLighting(WriteScript("status-fail", "exit 2\n")));
    CHECK(!ReadCachedLighting(WriteScript("status-garbage", "echo nope\n")));
}

static void TestWriterCoalesces()
{
    std::string log_file = scratch + "/writer.log";
    std::string cli = WriteScript("writer-cli",
        "echo \"$*\" >> '" + log_file + "'\nsleep 0.05\n");
    std::vector<std::string> warnings;
    {
        LightingWriter writer(cli, Solid(0), [&](const std::string& m) { warnings.push_back(m); });
        // A burst of 50 frames, like an OpenRGB effect.
        for(uint32_t frame = 1; frame <= 50; frame++)
        {
            writer.Request(Solid(frame));
            std::this_thread::sleep_for(std::chrono::milliseconds(5));
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(800));
    }
    std::vector<std::string> calls = ReadLines(log_file);
    CHECK(!calls.empty());
    CHECK(calls.size() < 10);   // ~250 ms of frames at >= 150 ms spacing
    // The last frame always reaches the device.
    CHECK(!calls.empty() && calls.back() == "set leds " + LedsArgument(Solid(50)));
    CHECK(warnings.empty());

    // A failing CLI is reported, and the next request resends everything.
    std::string failing = WriteScript("failing-cli", "echo nope >&2; exit 2\n");
    std::vector<std::string> failed;
    {
        LightingWriter writer(failing, Solid(0), [&](const std::string& m) { failed.push_back(m); });
        writer.Request(Solid(1));
        std::this_thread::sleep_for(std::chrono::milliseconds(300));
    }
    CHECK(failed.size() == 1);
    CHECK(!failed.empty() && failed[0].find("nope") != std::string::npos);
}

static void TestOwnership()
{
    // Every write identifies OpenRGB, replacing any inherited value.
    std::string env_file = scratch + "/env.txt";
    std::string cli = WriteScript("env-cli",
        "printf '%s\\n' \"$SONICCORE_LIGHTING_SOURCE\" >> '" + env_file + "'\n");
    setenv("SONICCORE_LIGHTING_SOURCE", "soniccore", 1);
    {
        LightingWriter writer(cli, Solid(0), [](const std::string&) {});
        writer.Request(Solid(7));
        std::this_thread::sleep_for(std::chrono::milliseconds(400));
        CHECK(writer.Idle());
        CHECK(writer.Applied() == std::optional<LightingState>(Solid(7)));
    }
    unsetenv("SONICCORE_LIGHTING_SOURCE");
    CHECK(ReadLines(env_file) == (std::vector<std::string>{ "openrgb" }));

    // While SonicCore holds the lighting, refused frames are reported once.
    std::string held = WriteScript("held-cli",
        "echo 'inspection failed: lighting is held by SonicCore; release it' >&2; exit 2\n");
    std::vector<std::string> messages;
    {
        LightingWriter writer(held, Solid(0), [&](const std::string& m) { messages.push_back(m); });
        for(uint32_t frame = 1; frame <= 4; frame++)
        {
            writer.Request(Solid(frame));
            std::this_thread::sleep_for(std::chrono::milliseconds(200));
        }
        CHECK(!writer.Applied().has_value());
    }
    CHECK(messages.size() == 1);
    CHECK(!messages.empty() && messages[0].find("holds the headset lighting") != std::string::npos);

    // A state another client wrote is adopted: requesting it sends nothing.
    std::string log_file = scratch + "/adopt.log";
    std::string logging = WriteScript("adopt-cli", "echo \"$*\" >> '" + log_file + "'\n");
    {
        LightingWriter writer(logging, Solid(0), [](const std::string&) {});
        writer.Adopt(Solid(9));
        writer.Request(Solid(9));
        std::this_thread::sleep_for(std::chrono::milliseconds(300));
    }
    CHECK(ReadLines(log_file).empty());

    // The cache reader reports who wrote last and whether SonicCore holds it.
    std::string status = WriteScript("status-owner",
        "echo '{\"lighting_enabled\": true, \"logo_effect\": \"solid\", \"logo_speed\": \"0.5\","
        " \"lighting_source\": \"soniccore\", \"lighting_held\": true,"
        " \"logo_colors\": [\"#000000\",\"#000000\",\"#000000\",\"#000000\",\"#000000\"],"
        " \"ring_colors\": [\"#000000\",\"#000000\",\"#000000\",\"#000000\",\"#000000\"]}'\n");
    std::optional<CachedLighting> cached = ReadCachedStatus(status);
    CHECK(cached && cached->source == "soniccore" && cached->held);

    // The watcher keeps reporting readings until it is destroyed.
    int readings = 0;
    {
        CacheWatcher watcher(status, [&](const CachedLighting&) { readings++; });
        std::this_thread::sleep_for(std::chrono::milliseconds(2300));
    }
    CHECK(readings == 1);
}

int main(int argc, char** argv)
{
    if(argc != 2)
    {
        std::cerr << "usage: openrgb-plugin-test <scratch-dir>\n";
        return 2;
    }
    scratch = argv[1];

    TestColors();
    TestPlan();
    TestRunCommand();
    TestCachedLighting();
    TestWriterCoalesces();
    TestOwnership();

    if(failures > 0)
    {
        std::cerr << failures << " check(s) failed\n";
        return 1;
    }
    std::cout << "openrgb plugin logic tests passed\n";
    return 0;
}
