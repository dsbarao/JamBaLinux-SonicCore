#include "volume.h"
#include "executable.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

/* PulseAudio's 100% (PA_VOLUME_NORM). */
static constexpr double NORMAL_VOLUME = 65536.0;

static QString quoted(const QString& value)
{
    QString escaped = value;
    escaped.replace(QLatin1Char('\''), QLatin1String("'\"'\"'"));
    return QLatin1Char('\'') + escaped + QLatin1Char('\'');
}

OutputVolume::OutputVolume(QObject* parent)
    : QObject(parent)
{
    refreshSoon.setSingleShot(true);
    refreshSoon.setInterval(60);
    connect(&refreshSoon, &QTimer::timeout, this, &OutputVolume::refresh);

    connect(&subscriber, &QProcess::readyReadStandardOutput, this, [this]() {
        while(subscriber.canReadLine())
        {
            const QByteArray line = subscriber.readLine();
            /* Sinks and the server (a new default) matter; streams do not. */
            if(line.contains(" on sink #") || line.contains(" on server"))
            {
                refreshSoon.start();
            }
        }
    });
    connect(&subscriber, &QProcess::finished, this, [this]() {
        /* PipeWire restarted: listen again once it is back. */
        QTimer::singleShot(2000, this, &OutputVolume::startSubscriber);
    });
    startSubscriber();
    refresh();
}

OutputVolume::~OutputVolume()
{
    subscriber.disconnect(this);
    subscriber.kill();
    subscriber.waitForFinished(500);
}

void OutputVolume::startSubscriber()
{
    QString program;
    const QStringList arguments = Executable::shellArguments(QStringLiteral("exec pactl subscribe"), &program);
    Executable::endWithApp(&subscriber);
    subscriber.start(program, arguments);
    refreshSoon.start();
}

OutputVolume::Reading OutputVolume::parse(const QString& defaultName, const QByteArray& sinksJson)
{
    Reading reading;
    const QJsonArray sinks = QJsonDocument::fromJson(sinksJson).array();
    for(const QJsonValue& value : sinks)
    {
        const QJsonObject sink = value.toObject();
        if(sink.value(QStringLiteral("name")).toString() != defaultName)
        {
            continue;
        }
        reading.name = defaultName;
        reading.description = sink.value(QStringLiteral("description")).toString();
        /* pactl prints "(null)" for a sink without a description. */
        if(reading.description == QLatin1String("(null)"))
        {
            reading.description.clear();
        }
        reading.muted = sink.value(QStringLiteral("mute")).toBool();
        const QJsonObject channels = sink.value(QStringLiteral("volume")).toObject();
        double sum = 0;
        int count = 0;
        for(const QJsonValue& channel : channels)
        {
            sum += channel.toObject().value(QStringLiteral("value")).toDouble();
            count++;
        }
        reading.percent = count > 0 ? qRound(sum / count / NORMAL_VOLUME * 100.0) : 0;
        break;
    }
    return reading;
}

void OutputVolume::refresh()
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
        const int split = output.indexOf('\n');
        const QString defaultName = QString::fromUtf8(output.left(split)).trimmed();
        const Reading reading = parse(defaultName, split < 0 ? QByteArray() : output.mid(split + 1));
        /* While a change of ours is still being written, the slider shows
           the newer value; the next reading confirms it. */
        if(!writer && pendingWrites.isEmpty()
           && (reading.name != sinkName || reading.description != sinkDescription
               || reading.percent != volumePercent || reading.muted != isMuted))
        {
            sinkName = reading.name;
            sinkDescription = reading.description;
            volumePercent = reading.percent;
            isMuted = reading.muted;
            Q_EMIT changed();
        }
        if(refreshAgain)
        {
            refreshAgain = false;
            refreshSoon.start();
        }
    });
    QString program;
    const QStringList arguments = Executable::shellArguments(
        QStringLiteral("pactl get-default-sink && pactl -f json list sinks"), &program);
    Executable::endWithApp(reader);
    reader->start(program, arguments);
}

void OutputVolume::run(const QStringList& pactlArguments)
{
    /* Only the newest volume matters while a slider is dragged. */
    if(!pendingWrites.isEmpty() && pendingWrites.last().value(0) == pactlArguments.value(0))
    {
        pendingWrites.last() = pactlArguments;
    }
    else
    {
        pendingWrites.append(pactlArguments);
    }
    runNext();
}

void OutputVolume::runNext()
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

void OutputVolume::setPercent(int percent)
{
    if(sinkName.isEmpty())
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
    if(wasMuted)
    {
        run({ QStringLiteral("set-sink-mute"), sinkName, QStringLiteral("0") });
    }
    run({ QStringLiteral("set-sink-volume"), sinkName, QString::number(percent) + QLatin1Char('%') });
}

void OutputVolume::setMuted(bool muted)
{
    if(sinkName.isEmpty())
    {
        return;
    }
    if(muted != isMuted)
    {
        isMuted = muted;
        Q_EMIT changed();
    }
    run({ QStringLiteral("set-sink-mute"), sinkName, muted ? QStringLiteral("1") : QStringLiteral("0") });
}
