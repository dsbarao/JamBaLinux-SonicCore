// Logic tests for the app launcher: which media player is shown, how the
// output volume is read, and how commands report back.

#include "executable.h"
#include "media.h"
#include "radio.h"
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

static void TestRadioRanking()
{
    // Two answers (two tags) of Radio Browser's station search for reggae.
    const QByteArray reggae = R"json([
        {"stationuuid":"generalist","name":"Big Radio 6","url_resolved":"https://big.example/stream",
         "favicon":"https://big.example/logo.png","countrycode":"GB","codec":"MP3","bitrate":128,
         "votes":3600,"clickcount":200,"lastcheckok":1,
         "tags":"alternative,indie,rock,pop,jazz,funk,soul,hip hop,electronic,reggae,folk,blues"},
        {"stationuuid":"devoted","name":" Roots  Reggae FM ","url_resolved":"https://roots.example/live",
         "favicon":"","countrycode":"JM","codec":"MP3","bitrate":128,
         "votes":50,"clickcount":40,"lastcheckok":1,"tags":"reggae,roots reggae"},
        {"stationuuid":"dead","name":"Dead","url_resolved":"https://dead.example","votes":999,
         "lastcheckok":0,"bitrate":128,"tags":"reggae"},
        {"stationuuid":"thin","name":"Thin","url_resolved":"https://thin.example","votes":900,
         "lastcheckok":1,"bitrate":24,"tags":"reggae"},
        {"stationuuid":"noaddress","name":"No address","url_resolved":"","url":"","votes":800,"lastcheckok":1},
        {"stationuuid":"spam","name":"# TOP 100 DJ CHARTS - DJ REMIX & CHARTS RADIO @ Reggaeton, Urban, HipHop, Club & Party",
         "url_resolved":"https://spam.example","votes":900,"clickcount":50,"lastcheckok":1,"tags":"reggae"}
    ])json";
    const QByteArray roots = R"json([
        {"stationuuid":"devoted","name":"Roots Reggae FM","url_resolved":"https://roots.example/live","votes":50,"lastcheckok":1},
        {"stationuuid":"copy","name":"roots reggae fm","url_resolved":"https://mirror.example/live","votes":10,"lastcheckok":1,"tags":"reggae"},
        {"stationuuid":"fallback","name":"Dub Station","url_resolved":"","url":"http://dub.example/live",
         "votes":5,"clickcount":3,"lastcheckok":1,"bitrate":0,"tags":"roots reggae,dub"},
        {"stationuuid":"odd","name":"Odd","url_resolved":"rtsp://odd.example","votes":60,"lastcheckok":1,"tags":"reggae"}
    ])json";
    const QStringList style = { QStringLiteral("reggae"), QStringLiteral("roots reggae") };
    const QVariantList stations = RadioDirectory::rank({ reggae, roots, "not json" }, style, 10);
    // Working, listenable, http(s) stations, each once (by id and name).
    CHECK(stations.size() == 3);
    // A reggae station comes before a big station that also plays reggae.
    CHECK(stations.value(0).toMap().value(QStringLiteral("uuid")) == QStringLiteral("devoted"));
    CHECK(stations.value(0).toMap().value(QStringLiteral("name")) == QStringLiteral("Roots Reggae FM"));
    CHECK(stations.value(0).toMap().value(QStringLiteral("country")) == QStringLiteral("JM"));
    QStringList order;
    for(const QVariant& station : stations)
    {
        order.append(station.toMap().value(QStringLiteral("uuid")).toString());
    }
    CHECK(order.contains(QStringLiteral("generalist")) && order.contains(QStringLiteral("fallback")));
    // `url` stands in for a missing `url_resolved`.
    for(const QVariant& station : stations)
    {
        if(station.toMap().value(QStringLiteral("uuid")) == QStringLiteral("fallback"))
        {
            CHECK(station.toMap().value(QStringLiteral("url")) == QStringLiteral("http://dub.example/live"));
        }
    }
    CHECK(RadioDirectory::rank({ reggae, roots }, style, 1).size() == 1);
}

static void TestRadioTitle()
{
    CHECK(RadioTitle::parse("StreamTitle='Depeche Mode - Waiting for the Night';StreamUrl='';")
          == QStringLiteral("Depeche Mode - Waiting for the Night"));
    CHECK(RadioTitle::parse("StreamTitle='Rock 'n' Roll';") == QStringLiteral("Rock 'n' Roll"));
    CHECK(RadioTitle::parse(QByteArray("StreamTitle='Jo\xe3o Gilberto';")) == QStringLiteral("João Gilberto"));
    CHECK(RadioTitle::parse("StreamTitle='Caf\xc3\xa9 Tacvba';") == QStringLiteral("Café Tacvba"));
    CHECK(RadioTitle::parse("StreamUrl='x';").isEmpty());
}

int main(int argc, char** argv)
{
    QCoreApplication app(argc, argv);
    TestPlayerChoice();
    TestVolumeReading();
    TestExecutable();
    TestRadioRanking();
    TestRadioTitle();
    if(failures > 0)
    {
        std::cerr << failures << " check(s) failed\n";
        return 1;
    }
    std::cout << "launcher logic tests passed\n";
    return 0;
}
