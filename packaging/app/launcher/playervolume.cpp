#include "playervolume.h"
#include "executable.h"

#include <QFile>
#include <QHash>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

#include <algorithm>

/* PulseAudio's 100% (PA_VOLUME_NORM). */
static constexpr double NORMAL_VOLUME = 65536.0;

static QString quoted(const QString& value)
{
    QString escaped = value;
    escaped.replace(QLatin1Char('\''), QLatin1String("'\"'\"'"));
    return QLatin1Char('\'') + escaped + QLatin1Char('\'');
}

/* A process's parent, from /proc (0 when it cannot be read, as in a
   Flatpak, where the host's processes are not visible). */
static qint64 parentOf(qint64 pid)
{
    QFile stat(QStringLiteral("/proc/%1/stat").arg(pid));
    if(!stat.open(QIODevice::ReadOnly))
    {
        return 0;
    }
    const QByteArray line = stat.readAll();
    /* "pid (name) state ppid ...": the name may hold spaces and ")". */
    const int close = line.lastIndexOf(')');
    const QList<QByteArray> fields = line.mid(close + 2).split(' ');
    return fields.size() > 1 ? fields.at(1).toLongLong() : 0;
}

PlayerVolume::PlayerVolume(QObject* parent)
    : QObject(parent)
{
    refreshSoon.setSingleShot(true);
    refreshSoon.setInterval(80);
    connect(&refreshSoon, &QTimer::timeout, this, &PlayerVolume::refresh);
    connect(&subscriber, &QProcess::readyReadStandardOutput, this, [this]() {
        while(subscriber.canReadLine())
        {
            /* Streams come, go and change volume. */
            if(subscriber.readLine().contains(" on sink-input #"))
            {
                refreshSoon.start();
            }
        }
    });
    connect(&subscriber, &QProcess::finished, this, [this]() {
        QTimer::singleShot(2000, this, &PlayerVolume::startSubscriber);
    });
    startSubscriber();
}

PlayerVolume::~PlayerVolume()
{
    subscriber.disconnect(this);
    subscriber.kill();
    subscriber.waitForFinished(500);
}

void PlayerVolume::startSubscriber()
{
    QString program;
    const QStringList arguments = Executable::shellArguments(QStringLiteral("exec pactl subscribe"), &program);
    Executable::endWithApp(&subscriber);
    subscriber.start(program, arguments);
    refreshSoon.start();
}

void PlayerVolume::setPid(qint64 value)
{
    if(value != playerPid)
    {
        playerPid = value;
        Q_EMIT targetChanged();
        refreshSoon.start();
    }
}

void PlayerVolume::setNames(const QStringList& value)
{
    if(value != playerNames)
    {
        playerNames = value;
        Q_EMIT targetChanged();
        refreshSoon.start();
    }
}

/* The separator between the two pactl answers read at once. */
static const QByteArray CLIENTS_MARK = "--clients--";

QList<PlayerVolume::Stream> PlayerVolume::parse(const QByteArray& json, const QByteArray& clientsJson)
{
    QHash<QString, QJsonObject> clients;
    for(const QJsonValue& value : QJsonDocument::fromJson(clientsJson).array())
    {
        const QJsonObject client = value.toObject();
        clients.insert(QString::number(client.value(QStringLiteral("index")).toInteger()),
                       client.value(QStringLiteral("properties")).toObject());
    }
    QList<Stream> streams;
    for(const QJsonValue& value : QJsonDocument::fromJson(json).array())
    {
        const QJsonObject input = value.toObject();
        const QJsonObject properties = input.value(QStringLiteral("properties")).toObject();
        Stream stream;
        stream.index = input.value(QStringLiteral("index")).toInt(-1);
        stream.pid = properties.value(QStringLiteral("application.process.id")).toString().toLongLong();
        stream.binary = properties.value(QStringLiteral("application.process.binary")).toString();
        stream.app = properties.value(QStringLiteral("application.name")).toString();
        if(stream.pid <= 0)
        {
            const QJsonValue clientIndex = input.value(QStringLiteral("client"));
            const QJsonObject client = clients.value(clientIndex.isString()
                ? clientIndex.toString() : QString::number(clientIndex.toInteger()));
            stream.pid = client.value(QStringLiteral("application.process.id")).toString().toLongLong();
            if(stream.pid <= 0)
            {
                stream.pid = client.value(QStringLiteral("pipewire.sec.pid")).toString().toLongLong();
            }
            if(stream.binary.isEmpty())
            {
                stream.binary = client.value(QStringLiteral("application.process.binary")).toString();
            }
        }
        stream.muted = input.value(QStringLiteral("mute")).toBool();
        double sum = 0;
        int count = 0;
        for(const QJsonValue& channel : input.value(QStringLiteral("volume")).toObject())
        {
            sum += channel.toObject().value(QStringLiteral("value")).toDouble();
            count++;
        }
        stream.percent = count > 0 ? qRound(sum / count / NORMAL_VOLUME * 100.0) : 0;
        if(stream.index >= 0)
        {
            streams.append(stream);
        }
    }
    return streams;
}

QList<int> PlayerVolume::match(const QList<Stream>& streams, qint64 pid, const QStringList& names)
{
    QList<int> found;
    if(pid > 0)
    {
        for(int position = 0; position < streams.size(); position++)
        {
            const Stream& stream = streams.at(position);
            if(stream.pid == pid || stream.ancestors.contains(pid))
            {
                found.append(position);
            }
        }
    }
    if(!found.isEmpty())
    {
        return found;
    }
    QStringList wanted;
    for(const QString& name : names)
    {
        const QString lower = name.trimmed().toLower();
        if(lower.size() >= 3)
        {
            wanted.append(lower);
        }
    }
    for(int position = 0; position < streams.size(); position++)
    {
        const Stream& stream = streams.at(position);
        const QString app = stream.app.toLower();
        const QString binary = stream.binary.toLower();
        /* PipeWire's own filters (the equalizer and the 3D sound) are not
           players. */
        if(binary == QLatin1String("pipewire") || binary == QLatin1String("wireplumber"))
        {
            continue;
        }
        const bool named = std::any_of(wanted.begin(), wanted.end(), [&](const QString& name) {
            return (app.size() >= 3 && (name == app || name.contains(app) || app.contains(name)))
                || (binary.size() >= 3 && name.contains(binary));
        });
        if(named)
        {
            found.append(position);
        }
    }
    return found;
}

void PlayerVolume::refresh()
{
    if(reader)
    {
        refreshAgain = true;
        return;
    }
    reader = new QProcess(this);
    connect(reader, &QProcess::finished, this, [this]() {
        const QByteArray output = reader->readAllStandardOutput();
        reader->deleteLater();
        reader = nullptr;
        const int mark = output.indexOf(CLIENTS_MARK);
        QList<Stream> streams = mark < 0 ? parse(output)
                                         : parse(output.left(mark), output.mid(mark + CLIENTS_MARK.size()));
        for(Stream& stream : streams)
        {
            qint64 parent = stream.pid;
            for(int depth = 0; depth < 16 && parent > 1; depth++)
            {
                parent = parentOf(parent);
                if(parent > 1)
                {
                    stream.ancestors.append(parent);
                }
            }
        }
        apply(streams);
        if(refreshAgain)
        {
            refreshAgain = false;
            refreshSoon.start();
        }
    });
    QString program;
    const QStringList arguments = Executable::shellArguments(
        QStringLiteral("pactl -f json list sink-inputs && echo --clients-- && pactl -f json list clients"), &program);
    Executable::endWithApp(reader);
    reader->start(program, arguments);
}

void PlayerVolume::apply(const QList<Stream>& streams)
{
    QList<int> indexes;
    int sum = 0;
    bool allMuted = true;
    const QList<int> positions = (playerPid > 0 || !playerNames.isEmpty())
        ? match(streams, playerPid, playerNames) : QList<int>();
    for(int position : positions)
    {
        const Stream& stream = streams.at(position);
        indexes.append(stream.index);
        sum += stream.percent;
        allMuted = allMuted && stream.muted;
    }
    const int percent = indexes.isEmpty() ? 0 : qRound(double(sum) / indexes.size());
    const bool muted = !indexes.isEmpty() && allMuted;
    /* While a change of ours is being written, the slider keeps it. */
    const bool writing = writer || !pendingWrites.isEmpty();
    if(indexes != matched || (!writing && (percent != volumePercent || muted != isMuted)))
    {
        matched = indexes;
        if(!writing)
        {
            volumePercent = percent;
            isMuted = muted;
        }
        Q_EMIT changed();
    }
}

void PlayerVolume::run(const QStringList& pactlArguments)
{
    /* Only the newest value matters while the slider is dragged. */
    for(QStringList& pending : pendingWrites)
    {
        if(pending.value(0) == pactlArguments.value(0) && pending.value(1) == pactlArguments.value(1))
        {
            pending = pactlArguments;
            runNext();
            return;
        }
    }
    pendingWrites.append(pactlArguments);
    runNext();
}

void PlayerVolume::runNext()
{
    if(writer || pendingWrites.isEmpty())
    {
        return;
    }
    const QStringList pactlArguments = pendingWrites.takeFirst();
    QString command = QStringLiteral("pactl");
    for(const QString& argument : pactlArguments)
    {
        command += QLatin1Char(' ') + quoted(argument);
    }
    writer = new QProcess(this);
    connect(writer, &QProcess::finished, this, [this]() {
        writer->deleteLater();
        writer = nullptr;
        if(pendingWrites.isEmpty())
        {
            refreshSoon.start();
        }
        runNext();
    });
    QString program;
    const QStringList arguments = Executable::shellArguments(command, &program);
    Executable::endWithApp(writer);
    writer->start(program, arguments);
}

void PlayerVolume::setPercent(int percent)
{
    if(matched.isEmpty())
    {
        return;
    }
    percent = qBound(0, percent, 100);
    const bool wasMuted = isMuted;
    if(percent != volumePercent || wasMuted)
    {
        volumePercent = percent;
        isMuted = false;
        Q_EMIT changed();
    }
    for(int index : std::as_const(matched))
    {
        if(wasMuted)
        {
            run({ QStringLiteral("set-sink-input-mute"), QString::number(index), QStringLiteral("0") });
        }
        run({ QStringLiteral("set-sink-input-volume"), QString::number(index), QString::number(percent) + QLatin1Char('%') });
    }
}

void PlayerVolume::setMuted(bool muted)
{
    if(matched.isEmpty())
    {
        return;
    }
    if(muted != isMuted)
    {
        isMuted = muted;
        Q_EMIT changed();
    }
    for(int index : std::as_const(matched))
    {
        run({ QStringLiteral("set-sink-input-mute"), QString::number(index), muted ? QStringLiteral("1") : QStringLiteral("0") });
    }
}
