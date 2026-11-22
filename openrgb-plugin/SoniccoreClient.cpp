/*---------------------------------------------------------*\
| SoniccoreClient.cpp                                       |
|                                                           |
|   SPDX-License-Identifier: MIT                            |
\*---------------------------------------------------------*/

#include "SoniccoreClient.h"

#include <cerrno>
#include <csignal>
#include <cstdlib>
#include <fcntl.h>
#include <poll.h>
#include <spawn.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include <QByteArray>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>

extern char** environ;

namespace jambalinux
{

static bool IsExecutable(const std::string& path)
{
    struct stat info;
    return !path.empty() && stat(path.c_str(), &info) == 0 && S_ISREG(info.st_mode)
        && access(path.c_str(), X_OK) == 0;
}

std::string ResolveSoniccore()
{
    const char* cargo_home = std::getenv("CARGO_HOME");
    if(cargo_home && *cargo_home)
    {
        std::string candidate = std::string(cargo_home) + "/bin/soniccore";
        if(IsExecutable(candidate))
        {
            return candidate;
        }
    }

    const char* home = std::getenv("HOME");
    if(home && *home)
    {
        std::string candidate = std::string(home) + "/.cargo/bin/soniccore";
        if(IsExecutable(candidate))
        {
            return candidate;
        }
    }

    const char* path = std::getenv("PATH");
    std::string directories = path ? path : "";
    std::size_t start = 0;
    while(start <= directories.size())
    {
        std::size_t end = directories.find(':', start);
        if(end == std::string::npos)
        {
            end = directories.size();
        }
        std::string directory = directories.substr(start, end - start);
        if(!directory.empty() && IsExecutable(directory + "/soniccore"))
        {
            return directory + "/soniccore";
        }
        start = end + 1;
    }

    return "";
}

static void ReadAvailable(int fd, std::string& into)
{
    char buffer[4096];
    for(;;)
    {
        ssize_t count = read(fd, buffer, sizeof(buffer));
        if(count > 0)
        {
            /* Status output is small; cap it to stay bounded. */
            if(into.size() < 1024 * 1024)
            {
                into.append(buffer, (std::size_t)count);
            }
            continue;
        }
        if(count < 0 && errno == EINTR)
        {
            continue;
        }
        return;
    }
}

CommandResult RunCommand(const std::string& program, const Command& arguments,
                         std::chrono::milliseconds timeout,
                         const std::vector<std::string>& extra_environment)
{
    CommandResult result;
    if(program.empty())
    {
        result.err = "soniccore not found";
        return result;
    }

    int out_pipe[2];
    int err_pipe[2];
    if(pipe2(out_pipe, O_CLOEXEC | O_NONBLOCK) != 0)
    {
        result.err = "could not create a pipe";
        return result;
    }
    if(pipe2(err_pipe, O_CLOEXEC | O_NONBLOCK) != 0)
    {
        close(out_pipe[0]);
        close(out_pipe[1]);
        result.err = "could not create a pipe";
        return result;
    }

    posix_spawn_file_actions_t actions;
    posix_spawn_file_actions_init(&actions);
    posix_spawn_file_actions_addopen(&actions, STDIN_FILENO, "/dev/null", O_RDONLY, 0);
    posix_spawn_file_actions_adddup2(&actions, out_pipe[1], STDOUT_FILENO);
    posix_spawn_file_actions_adddup2(&actions, err_pipe[1], STDERR_FILENO);

    /* Argument vector only: values are never parsed by a shell. */
    std::vector<char*> argv;
    argv.push_back(const_cast<char*>(program.c_str()));
    for(const std::string& argument : arguments)
    {
        argv.push_back(const_cast<char*>(argument.c_str()));
    }
    argv.push_back(nullptr);

    /* The parent environment, with extra entries replacing same-named ones. */
    std::vector<char*> envp;
    for(char** entry = environ; entry && *entry; entry++)
    {
        std::string current(*entry);
        bool replaced = false;
        for(const std::string& extra : extra_environment)
        {
            std::string name = extra.substr(0, extra.find('=') + 1);
            if(current.rfind(name, 0) == 0)
            {
                replaced = true;
                break;
            }
        }
        if(!replaced)
        {
            envp.push_back(*entry);
        }
    }
    for(const std::string& extra : extra_environment)
    {
        envp.push_back(const_cast<char*>(extra.c_str()));
    }
    envp.push_back(nullptr);

    pid_t pid = 0;
    int spawn_error = posix_spawn(&pid, program.c_str(), &actions, nullptr, argv.data(), envp.data());
    posix_spawn_file_actions_destroy(&actions);
    close(out_pipe[1]);
    close(err_pipe[1]);

    if(spawn_error != 0)
    {
        close(out_pipe[0]);
        close(err_pipe[0]);
        result.err = "could not start soniccore";
        return result;
    }

    const auto deadline = std::chrono::steady_clock::now() + timeout;
    int status = 0;
    bool exited = false;
    for(;;)
    {
        ReadAvailable(out_pipe[0], result.out);
        ReadAvailable(err_pipe[0], result.err);

        pid_t waited = waitpid(pid, &status, WNOHANG);
        if(waited == pid)
        {
            exited = true;
            break;
        }
        if(waited < 0 && errno != EINTR)
        {
            break;
        }
        if(std::chrono::steady_clock::now() >= deadline)
        {
            kill(pid, SIGKILL);
            waitpid(pid, &status, 0);
            result.err += "\nsoniccore timed out";
            break;
        }

        struct pollfd fds[2] = { { out_pipe[0], POLLIN, 0 }, { err_pipe[0], POLLIN, 0 } };
        poll(fds, 2, 20);
    }

    ReadAvailable(out_pipe[0], result.out);
    ReadAvailable(err_pipe[0], result.err);
    close(out_pipe[0]);
    close(err_pipe[0]);

    if(exited && WIFEXITED(status))
    {
        result.exit_code = WEXITSTATUS(status);
    }
    return result;
}

static std::optional<int> ModeFromEffect(const std::string& effect)
{
    for(int mode = MODE_DIRECT; mode < MODE_OFF; mode++)
    {
        if(effect == EFFECT_NAMES[mode])
        {
            return mode;
        }
    }
    return std::nullopt;
}

static unsigned int SpeedFromName(const std::string& speed)
{
    for(unsigned int index = 0; index <= SPEED_MAX; index++)
    {
        if(speed == SPEED_NAMES[index])
        {
            return index;
        }
    }
    return 0;
}

std::optional<CachedLighting> ReadCachedStatus(const std::string& program)
{
    CommandResult result = RunCommand(program, { "cached-status", "--format", "json" },
                                      std::chrono::milliseconds(3000));
    if(result.exit_code != 0)
    {
        return std::nullopt;
    }

    QJsonDocument document = QJsonDocument::fromJson(QByteArray::fromStdString(result.out));
    if(!document.isObject())
    {
        return std::nullopt;
    }
    QJsonObject status = document.object();

    LightingState state;
    QJsonArray logo = status.value("logo_colors").toArray();
    QJsonArray ring = status.value("ring_colors").toArray();
    if(logo.size() != 5 || ring.size() != 5)
    {
        return std::nullopt;
    }
    for(unsigned int index = 0; index < LED_COUNT; index++)
    {
        QJsonValue value = index < 5 ? logo.at(index) : ring.at(index - 5);
        std::optional<uint32_t> color = value.isString()
            ? ParseColorHex(value.toString().toStdString()) : std::nullopt;
        if(!color)
        {
            return std::nullopt;
        }
        state.colors[index] = *color;
    }

    QJsonValue effect = status.value("logo_effect");
    std::optional<int> mode = effect.isString()
        ? ModeFromEffect(effect.toString().toStdString()) : std::nullopt;
    if(!mode)
    {
        return std::nullopt;
    }
    state.mode = *mode;

    QJsonValue speed = status.value("logo_speed");
    state.speed = speed.isString() ? SpeedFromName(speed.toString().toStdString()) : 0;

    QJsonValue enabled = status.value("lighting_enabled");
    if(enabled.isBool() && !enabled.toBool())
    {
        state.mode = MODE_OFF;
    }

    CachedLighting cached;
    cached.state  = state;
    cached.source = status.value("lighting_source").toString().toStdString();
    cached.held   = status.value("lighting_held").toBool(false);
    return cached;
}

std::optional<LightingState> ReadCachedLighting(const std::string& program)
{
    std::optional<CachedLighting> cached = ReadCachedStatus(program);
    if(!cached)
    {
        return std::nullopt;
    }
    return cached->state;
}

static std::string RequestedLightingPath()
{
    const char* runtime = std::getenv("XDG_RUNTIME_DIR");
    if(runtime == nullptr || runtime[0] != '/')
    {
        return std::string();
    }
    return std::string(runtime) + "/jambalinux-soniccore-openrgb.json";
}

void PublishRequestedLighting(const LightingState& state)
{
    const std::string path = RequestedLightingPath();
    if(path.empty())
    {
        return;
    }
    QJsonArray colors;
    for(uint32_t color : state.colors)
    {
        colors.append(QString::fromStdString(ColorHex(color)));
    }
    QJsonObject object;
    object["schema"] = 1;
    object["off"]    = state.mode == MODE_OFF;
    object["colors"] = colors;
    const QByteArray text = QJsonDocument(object).toJson(QJsonDocument::Compact);

    /* Written aside and renamed, so a reader never sees half a file. */
    const std::string temporary = path + ".tmp";
    int fd = open(temporary.c_str(), O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600);
    if(fd < 0)
    {
        return;
    }
    bool written = write(fd, text.constData(), text.size()) == text.size();
    written = close(fd) == 0 && written;
    if(!written || rename(temporary.c_str(), path.c_str()) != 0)
    {
        unlink(temporary.c_str());
    }
}

void ClearRequestedLighting()
{
    const std::string path = RequestedLightingPath();
    if(!path.empty())
    {
        unlink(path.c_str());
    }
}

LightingWriter::LightingWriter(std::string program, std::optional<LightingState> applied, Logger log)
    : program(std::move(program)), applied(std::move(applied)), log(std::move(log))
{
    worker = std::thread(&LightingWriter::Run, this);
}

LightingWriter::~LightingWriter()
{
    {
        std::lock_guard<std::mutex> lock(mutex);
        stopping = true;
    }
    wake.notify_all();
    if(worker.joinable())
    {
        worker.join();
    }
    ClearRequestedLighting();
}

void LightingWriter::Request(const LightingState& state)
{
    {
        std::lock_guard<std::mutex> lock(mutex);
        pending = state;
    }
    wake.notify_all();
}

bool LightingWriter::Idle()
{
    std::lock_guard<std::mutex> lock(mutex);
    return !busy && !pending.has_value();
}

std::optional<LightingState> LightingWriter::Applied()
{
    std::lock_guard<std::mutex> lock(mutex);
    return applied;
}

void LightingWriter::Adopt(const LightingState& state)
{
    std::lock_guard<std::mutex> lock(mutex);
    applied = state;
}

void LightingWriter::Run()
{
    const std::vector<std::string> environment = {
        std::string(LIGHTING_SOURCE_ENV) + "=" + LIGHTING_SOURCE_VALUE
    };

    for(;;)
    {
        LightingState desired;
        std::optional<LightingState> known;
        {
            std::unique_lock<std::mutex> lock(mutex);
            wake.wait(lock, [this] { return stopping || pending.has_value(); });
            if(stopping)
            {
                return;
            }
            desired = *pending;
            pending.reset();
            known = applied;
            busy = true;
        }

        /* Before the device write, which fails while the headset is off. */
        PublishRequestedLighting(desired);

        bool ok   = true;
        bool held = false;
        for(const Command& command : PlanCommands(known, desired))
        {
            CommandResult result = RunCommand(program, command, COMMAND_TIMEOUT, environment);
            if(result.exit_code != 0)
            {
                ok   = false;
                held = result.err.find(LIGHTING_HELD_MARKER) != std::string::npos;
                if(!held)
                {
                    std::string joined;
                    for(const std::string& part : command)
                    {
                        joined += " " + part;
                    }
                    log("soniccore" + joined + " failed: " + result.err);
                }
                break;
            }
        }

        {
            std::lock_guard<std::mutex> lock(mutex);
            /* After a failure the device state is unknown: resend everything next time. */
            applied = ok ? std::optional<LightingState>(desired) : std::nullopt;
            busy    = false;
            /* Report the hold once, not on every refused effect frame. */
            if(held && !held_reported)
            {
                held_reported = true;
                log("SonicCore holds the headset lighting; OpenRGB changes are paused "
                    "until it is released in the SonicCore widget");
            }
            else if(ok)
            {
                held_reported = false;
            }
        }

        std::unique_lock<std::mutex> lock(mutex);
        wake.wait_for(lock, MIN_INTERVAL, [this] { return stopping; });
        if(stopping)
        {
            return;
        }
    }
}

CacheWatcher::CacheWatcher(std::string program, Callback callback)
    : program(std::move(program)), callback(std::move(callback))
{
    worker = std::thread(&CacheWatcher::Run, this);
}

CacheWatcher::~CacheWatcher()
{
    {
        std::lock_guard<std::mutex> lock(mutex);
        stopping = true;
    }
    wake.notify_all();
    if(worker.joinable())
    {
        worker.join();
    }
}

void CacheWatcher::Run()
{
    for(;;)
    {
        {
            std::unique_lock<std::mutex> lock(mutex);
            wake.wait_for(lock, INTERVAL, [this] { return stopping; });
            if(stopping)
            {
                return;
            }
        }
        std::optional<CachedLighting> cached = ReadCachedStatus(program);
        if(cached)
        {
            callback(*cached);
        }
    }
}

}
