/*---------------------------------------------------------*\
| SoniccoreClient.h                                         |
|                                                           |
|   Runs the soniccore CLI without a shell (argument       |
|   vectors only) and serializes lighting writes on one    |
|   background thread that always applies the newest       |
|   requested state.                                        |
|                                                           |
|   SPDX-License-Identifier: MIT                            |
\*---------------------------------------------------------*/

#pragma once

#include <chrono>
#include <condition_variable>
#include <functional>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

#include "LightingPlan.h"

namespace jambalinux
{

struct CommandResult
{
    int             exit_code = -1;     /* -1: could not run or timed out */
    std::string     out;
    std::string     err;
};

/* Absolute soniccore path: $CARGO_HOME/bin, then ~/.cargo/bin, then PATH. */
std::string ResolveSoniccore();

CommandResult RunCommand(const std::string& program, const Command& arguments,
                         std::chrono::milliseconds timeout);

/* Reads the lighting state the daemon cached, if it is complete. */
std::optional<LightingState> ReadCachedLighting(const std::string& program);

class LightingWriter
{
public:
    typedef std::function<void(const std::string&)> Logger;

    LightingWriter(std::string program, std::optional<LightingState> applied, Logger log);
    ~LightingWriter();

    /* Replaces any state still waiting; never blocks on the device. */
    void Request(const LightingState& state);

private:
    void Run();

    /*-----------------------------------------------------*\
    | Each lighting write sends a full profile (13 reports) |
    | to the headset. Effects plugins can request dozens of |
    | frames per second, so writes are spaced at least this |
    | far apart; intermediate frames are dropped and the    |
    | newest state is always the one sent.                  |
    \*-----------------------------------------------------*/
    static constexpr std::chrono::milliseconds MIN_INTERVAL{150};
    static constexpr std::chrono::milliseconds COMMAND_TIMEOUT{10000};

    std::string                     program;
    std::optional<LightingState>    applied;
    Logger                          log;

    std::mutex                      mutex;
    std::condition_variable         wake;
    std::optional<LightingState>    pending;
    bool                            stopping = false;
    std::thread                     worker;
};

}
