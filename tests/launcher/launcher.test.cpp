// Logic tests for the app launcher: which media player is shown, how the
// output volume is read, and how commands report back.

#include "executable.h"
#include "media.h"
#include "volume.h"

#include <QCoreApplication>
#include <QEventLoop>
#include <QTimer>
#include <iostream>

static int failures = 0;

#define CHECK(condition)                                                        \
    do {                                                                        \
        if(!(condition)) {                                                      \
            std::cerr << __FILE__ << ":" << __LINE__ << ": " #condition "\n";   \
            failures++;                                                         \
        }                                                                       \
    } while(false)

static MediaController::Player player(const char* status, qint64 activeSince)
{
    MediaController::Player result;
    result.status = QString::fromLatin1(status);
    result.activeSince = activeSince;
    return result;
}

static void TestPlayerChoice()
{
    QHash<QString, MediaController::Player> players;
    CHECK(MediaController::choose(players, QString()).isEmpty());

    players.insert(QStringLiteral("browser"), player("Paused", 10));
    players.insert(QStringLiteral("spotify"), player("Playing", 20));
    players.insert(QStringLiteral("vlc"), player("Playing", 30));
    // The one that started playing last.
    CHECK(MediaController::choose(players, QStringLiteral("browser")) == QStringLiteral("vlc"));

    // Nothing plays: the one shown stays while it exists...
    players[QStringLiteral("spotify")].status = QStringLiteral("Paused");
    players[QStringLiteral("vlc")].status = QStringLiteral("Stopped");
    CHECK(MediaController::choose(players, QStringLiteral("spotify")) == QStringLiteral("spotify"));
    // ...else the last one that was active.
    CHECK(MediaController::choose(players, QStringLiteral("gone")) == QStringLiteral("vlc"));
}

static void TestVolumeReading()
{
    const QByteArray sinks = R"([
        {"name":"hdmi","description":"HDMI","mute":false,
         "volume":{"front-left":{"value":65536},"front-right":{"value":65536}}},
        {"name":"speakers","description":"Caixas","mute":true,
         "volume":{"front-left":{"value":26214,"value_percent":"40%"},"front-right":{"value":32768}}}
    ])";
    const OutputVolume::Reading speakers = OutputVolume::parse(QStringLiteral("speakers"), sinks);
    CHECK(speakers.name == QStringLiteral("speakers"));
    CHECK(speakers.description == QStringLiteral("Caixas"));
    CHECK(speakers.muted);
    CHECK(speakers.percent == 45);   // the mean of 40% and 50%
    CHECK(OutputVolume::parse(QStringLiteral("hdmi"), sinks).percent == 100);
    const QByteArray unnamed = R"json([{"name":"odd","description":"(null)","mute":false,"volume":{}}])json";
    CHECK(OutputVolume::parse(QStringLiteral("odd"), unnamed).description.isEmpty());
    // A default that is not listed (or no PipeWire) is "unavailable".
    CHECK(OutputVolume::parse(QStringLiteral("missing"), sinks).name.isEmpty());
    CHECK(OutputVolume::parse(QStringLiteral("speakers"), "not json").name.isEmpty());
}

static QVariantMap runOnce(Executable& executable, const QString& command, int* starts)
{
    QVariantMap result;
    QEventLoop loop;
    auto connection = QObject::connect(&executable, &Executable::newData, &loop,
        [&](const QString& source, const QVariantMap& data) {
            if(source == command)
            {
                result = data;
                loop.quit();
            }
        });
    executable.connectSource(command);
    // Asking again while it runs does not start a second copy.
    executable.connectSource(command);
    *starts = executable.connectedSources().size();
    QTimer::singleShot(5000, &loop, &QEventLoop::quit);
    loop.exec();
    QObject::disconnect(connection);
    return result;
}

static void TestExecutable()
{
    Executable executable;
    int starts = 0;
    const QVariantMap data = runOnce(executable,
        QStringLiteral("echo 'olá'; echo erro >&2; exit 3"), &starts);
    CHECK(starts == 1);
    CHECK(data.value(QStringLiteral("exit code")).toInt() == 3);
    CHECK(data.value(QStringLiteral("stdout")).toString() == QStringLiteral("olá\n"));
    CHECK(data.value(QStringLiteral("stderr")).toString() == QStringLiteral("erro\n"));
    CHECK(executable.connectedSources().isEmpty());
}

int main(int argc, char** argv)
{
    QCoreApplication app(argc, argv);
    TestPlayerChoice();
    TestVolumeReading();
    TestExecutable();
    if(failures > 0)
    {
        std::cerr << failures << " check(s) failed\n";
        return 1;
    }
    std::cout << "launcher logic tests passed\n";
    return 0;
}
