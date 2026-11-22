#include "radiompris.h"

#include <QGuiApplication>
#include <QDBusConnection>
#include <QDBusMessage>

static const QString PATH = QStringLiteral("/org/mpris/MediaPlayer2");
static const QString PLAYER_IFACE = QStringLiteral("org.mpris.MediaPlayer2.Player");

RadioMpris::RadioMpris(QObject* parent)
    : QObject(parent)
    , serviceName(QStringLiteral("org.mpris.MediaPlayer2.jambalinux_soniccore"))
{
    const QString instance = qEnvironmentVariable("SONICCORE_INSTANCE");
    if(!instance.isEmpty())
    {
        serviceName += QStringLiteral(".instance_") + instance;
    }
    exported = new QObject(this);
    new RadioRootAdaptor(exported, this);
    new RadioPlayerAdaptor(exported, this);
}

RadioMpris::~RadioMpris()
{
    setActive(false);
}

void RadioMpris::setActive(bool active)
{
    if(active == published)
    {
        return;
    }
    QDBusConnection bus = QDBusConnection::sessionBus();
    if(active)
    {
        published = bus.registerObject(PATH, exported, QDBusConnection::ExportAdaptors)
            && bus.registerService(serviceName);
        if(!published)
        {
            bus.unregisterObject(PATH);
        }
    }
    else
    {
        bus.unregisterService(serviceName);
        bus.unregisterObject(PATH);
        published = false;
    }
    Q_EMIT activeChanged();
}

void RadioMpris::setPlaybackStatus(const QString& value)
{
    if(value == status)
    {
        return;
    }
    status = value;
    Q_EMIT playbackStatusChanged();
    announce({ { QStringLiteral("PlaybackStatus"), status } });
}

void RadioMpris::setTitle(const QString& value)
{
    if(value == songTitle)
    {
        return;
    }
    songTitle = value;
    Q_EMIT metadataChanged();
    announce({ { QStringLiteral("Metadata"), metadata() } });
}

void RadioMpris::setStation(const QString& value)
{
    if(value == stationName)
    {
        return;
    }
    stationName = value;
    track++;
    Q_EMIT metadataChanged();
    announce({ { QStringLiteral("Metadata"), metadata() } });
}

void RadioMpris::setArtUrl(const QString& value)
{
    if(value == art)
    {
        return;
    }
    art = value;
    Q_EMIT metadataChanged();
    announce({ { QStringLiteral("Metadata"), metadata() } });
}

QVariantMap RadioMpris::metadata() const
{
    QVariantMap fields;
    fields.insert(QStringLiteral("mpris:trackid"),
                  QVariant::fromValue(QDBusObjectPath(QStringLiteral("/org/jambalinux/soniccore/radio/%1").arg(track))));
    /* Song first when the station announces one, the station as artist. */
    fields.insert(QStringLiteral("xesam:title"), songTitle.isEmpty() ? stationName : songTitle);
    fields.insert(QStringLiteral("xesam:artist"), QStringList { songTitle.isEmpty() ? QStringLiteral("Rádio") : stationName });
    if(!art.isEmpty())
    {
        fields.insert(QStringLiteral("mpris:artUrl"), art);
    }
    return fields;
}

void RadioMpris::announce(const QVariantMap& changed)
{
    if(!published)
    {
        return;
    }
    QDBusMessage signal = QDBusMessage::createSignal(PATH, QStringLiteral("org.freedesktop.DBus.Properties"),
                                                     QStringLiteral("PropertiesChanged"));
    signal.setArguments({ PLAYER_IFACE, changed, QStringList() });
    QDBusConnection::sessionBus().send(signal);
}

RadioRootAdaptor::RadioRootAdaptor(QObject* exported, RadioMpris* radio)
    : QDBusAbstractAdaptor(exported)
    , radio(radio)
{
}

QString RadioRootAdaptor::identity() const
{
    return QStringLiteral("Rádio Sonic Core");
}

QString RadioRootAdaptor::desktopEntry() const
{
    return QGuiApplication::desktopFileName();
}

void RadioRootAdaptor::Raise()
{
    Q_EMIT radio->raiseRequested();
}

RadioPlayerAdaptor::RadioPlayerAdaptor(QObject* exported, RadioMpris* radio)
    : QDBusAbstractAdaptor(exported)
    , radio(radio)
{
}

QString RadioPlayerAdaptor::playbackStatus() const { return radio->playbackStatus(); }
QVariantMap RadioPlayerAdaptor::metadata() const { return radio->metadata(); }
void RadioPlayerAdaptor::Next() { Q_EMIT radio->nextRequested(); }
void RadioPlayerAdaptor::Previous() { Q_EMIT radio->previousRequested(); }
void RadioPlayerAdaptor::Pause() { Q_EMIT radio->pauseRequested(); }
void RadioPlayerAdaptor::PlayPause() { Q_EMIT radio->playPauseRequested(); }
void RadioPlayerAdaptor::Stop() { Q_EMIT radio->stopRequested(); }
void RadioPlayerAdaptor::Play() { Q_EMIT radio->playRequested(); }
