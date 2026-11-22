#include "media.h"
#include "dbushelpers.h"

#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusObjectPath>
#include <QDBusPendingCallWatcher>
#include <QDBusVariant>
#include <QDateTime>

static const QString MPRIS_PREFIX = QStringLiteral("org.mpris.MediaPlayer2.");
static const QString MPRIS_PATH = QStringLiteral("/org/mpris/MediaPlayer2");
static const QString ROOT_IFACE = QStringLiteral("org.mpris.MediaPlayer2");
static const QString PLAYER_IFACE = QStringLiteral("org.mpris.MediaPlayer2.Player");
static const QString PROPERTIES_IFACE = QStringLiteral("org.freedesktop.DBus.Properties");

MediaController::MediaController(QObject* parent)
    : QObject(parent)
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    bus.connect(QStringLiteral("org.freedesktop.DBus"), QStringLiteral("/org/freedesktop/DBus"),
                QStringLiteral("org.freedesktop.DBus"), QStringLiteral("NameOwnerChanged"),
                this, SLOT(nameOwnerChanged(QString,QString,QString)));
    bus.connect(QString(), MPRIS_PATH, PROPERTIES_IFACE, QStringLiteral("PropertiesChanged"),
                this, SLOT(propertiesChanged(QDBusMessage)));
    bus.connect(QString(), MPRIS_PATH, PLAYER_IFACE, QStringLiteral("Seeked"),
                this, SLOT(seeked(QDBusMessage)));
    positionPoll.setInterval(1000);
    connect(&positionPoll, &QTimer::timeout, this, &MediaController::fetchPosition);
    if(QDBusConnectionInterface* names = bus.interface())
    {
        for(const QString& name : names->registeredServiceNames().value())
        {
            if(name.startsWith(MPRIS_PREFIX))
            {
                addPlayer(name, names->serviceOwner(name).value());
            }
        }
    }
}

void MediaController::addPlayer(const QString& service, const QString& owner)
{
    Player player;
    player.owner = owner;
    players.insert(service, player);
    fetch(service, ROOT_IFACE);
    fetch(service, PLAYER_IFACE);
    /* Which process plays, for its volume ("Tocando agora"). */
    QDBusMessage message = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.DBus"), QStringLiteral("/org/freedesktop/DBus"),
        QStringLiteral("org.freedesktop.DBus"), QStringLiteral("GetConnectionUnixProcessID"));
    message.setArguments({ owner });
    auto* watcher = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(message, 2000), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, watcher, service, owner]() {
        watcher->deleteLater();
        const QDBusMessage reply = watcher->reply();
        auto found = players.find(service);
        if(found == players.end() || found->owner != owner || reply.type() != QDBusMessage::ReplyMessage
           || reply.arguments().isEmpty())
        {
            return;
        }
        found->pid = reply.arguments().first().toLongLong();
        update();
    });
}

void MediaController::fetch(const QString& service, const QString& iface)
{
    QDBusMessage message = QDBusMessage::createMethodCall(service, MPRIS_PATH, PROPERTIES_IFACE,
                                                          QStringLiteral("GetAll"));
    message.setArguments({ iface });
    auto* watcher = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(message, 2000), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, watcher, service]() {
        watcher->deleteLater();
        const QDBusMessage reply = watcher->reply();
        auto found = players.find(service);
        if(found == players.end() || reply.type() != QDBusMessage::ReplyMessage || reply.arguments().isEmpty())
        {
            return;
        }
        apply(*found, plainDBusValue(reply.arguments().first()).toMap());
        update();
    });
}

void MediaController::apply(Player& player, const QVariantMap& properties)
{
    for(auto entry = properties.cbegin(); entry != properties.cend(); ++entry)
    {
        const QString& key = entry.key();
        const QVariant value = plainDBusValue(entry.value());
        if(key == QLatin1String("Identity"))
        {
            player.identity = value.toString();
        }
        else if(key == QLatin1String("DesktopEntry"))
        {
            player.desktopEntry = value.toString();
        }
        else if(key == QLatin1String("Metadata"))
        {
            player.metadata = value.toMap();
        }
        else if(key == QLatin1String("PlaybackStatus"))
        {
            const QString status = value.toString();
            if(status == QLatin1String("Playing") && player.status != status)
            {
                player.activeSince = QDateTime::currentMSecsSinceEpoch();
            }
            player.status = status;
        }
        else if(key == QLatin1String("CanGoNext"))
        {
            player.canGoNext = value.toBool();
        }
        else if(key == QLatin1String("CanGoPrevious"))
        {
            player.canGoPrevious = value.toBool();
        }
        else if(key == QLatin1String("CanPlay"))
        {
            player.canPlay = value.toBool();
        }
        else if(key == QLatin1String("CanPause"))
        {
            player.canPause = value.toBool();
        }
        else if(key == QLatin1String("CanSeek"))
        {
            player.canSeek = value.toBool();
        }
        else if(key == QLatin1String("Position"))
        {
            player.position = value.toLongLong();
        }
    }
}

void MediaController::nameOwnerChanged(const QString& name, const QString&, const QString& newOwner)
{
    if(!name.startsWith(MPRIS_PREFIX))
    {
        return;
    }
    if(newOwner.isEmpty())
    {
        players.remove(name);
        update();
    }
    else
    {
        addPlayer(name, newOwner);
    }
}

void MediaController::propertiesChanged(const QDBusMessage& message)
{
    const QList<QVariant> arguments = message.arguments();
    if(arguments.size() < 2)
    {
        return;
    }
    const QString iface = arguments.at(0).toString();
    if(iface != PLAYER_IFACE && iface != ROOT_IFACE)
    {
        return;
    }
    for(auto player = players.begin(); player != players.end(); ++player)
    {
        if(player->owner != message.service())
        {
            continue;
        }
        apply(*player, plainDBusValue(arguments.at(1)).toMap());
        /* Properties sent without a value must be read again. */
        if(arguments.size() > 2 && !plainDBusValue(arguments.at(2)).toList().isEmpty())
        {
            fetch(player.key(), iface);
        }
        update();
        return;
    }
}

QString MediaController::choose(const QHash<QString, Player>& players, const QString& current)
{
    QString best;
    qint64 latest = -1;
    for(auto player = players.cbegin(); player != players.cend(); ++player)
    {
        if(player->status == QLatin1String("Playing") && player->activeSince > latest)
        {
            best = player.key();
            latest = player->activeSince;
        }
    }
    if(!best.isEmpty())
    {
        return best;
    }
    if(players.contains(current))
    {
        return current;
    }
    for(auto player = players.cbegin(); player != players.cend(); ++player)
    {
        if(best.isEmpty() || player->activeSince > latest)
        {
            best = player.key();
            latest = player->activeSince;
        }
    }
    return best;
}

void MediaController::update()
{
    current = choose(players, current);
    Q_EMIT changed();
    Q_EMIT positionChanged();
    if(playing())
    {
        if(!positionPoll.isActive())
        {
            positionPoll.start();
        }
    }
    else
    {
        positionPoll.stop();
    }
    /* Another player or another track: where it is now. */
    const QString shown = current + QLatin1Char('\n') + track();
    if(hasPlayer() && shown != shownTrack)
    {
        fetchPosition();
    }
    shownTrack = shown;
}

void MediaController::fetchPosition()
{
    if(!hasPlayer())
    {
        return;
    }
    const QString service = current;
    QDBusMessage message = QDBusMessage::createMethodCall(service, MPRIS_PATH, PROPERTIES_IFACE,
                                                          QStringLiteral("Get"));
    message.setArguments({ PLAYER_IFACE, QStringLiteral("Position") });
    auto* watcher = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(message, 1000), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, watcher, service]() {
        watcher->deleteLater();
        const QDBusMessage reply = watcher->reply();
        auto found = players.find(service);
        if(found == players.end() || reply.type() != QDBusMessage::ReplyMessage || reply.arguments().isEmpty())
        {
            return;
        }
        found->position = plainDBusValue(reply.arguments().first()).toLongLong();
        if(service == current)
        {
            Q_EMIT positionChanged();
        }
    });
}

void MediaController::seeked(const QDBusMessage& message)
{
    if(message.arguments().isEmpty())
    {
        return;
    }
    for(auto player = players.begin(); player != players.end(); ++player)
    {
        if(player->owner == message.service())
        {
            player->position = plainDBusValue(message.arguments().first()).toLongLong();
            if(player.key() == current)
            {
                Q_EMIT positionChanged();
            }
            return;
        }
    }
}

void MediaController::setPosition(double milliseconds)
{
    if(!canSeek())
    {
        return;
    }
    Player& player = players[current];
    const qint64 target = qBound<qint64>(0, qint64(milliseconds * 1000.0),
                                         qMax<qint64>(0, qint64(length() * 1000.0)));
    const QString trackId = player.metadata.value(QStringLiteral("mpris:trackid")).toString();
    QDBusMessage message;
    if(!trackId.isEmpty())
    {
        message = QDBusMessage::createMethodCall(current, MPRIS_PATH, PLAYER_IFACE, QStringLiteral("SetPosition"));
        message.setArguments({ QVariant::fromValue(QDBusObjectPath(trackId)), QVariant::fromValue(target) });
    }
    else
    {
        message = QDBusMessage::createMethodCall(current, MPRIS_PATH, PLAYER_IFACE, QStringLiteral("Seek"));
        message.setArguments({ QVariant::fromValue(target - player.position) });
    }
    QDBusConnection::sessionBus().asyncCall(message, 2000);
    /* Shown at once; the player's Seeked signal confirms it. */
    player.position = target;
    Q_EMIT positionChanged();
}

void MediaController::call(const QString& method)
{
    if(!hasPlayer())
    {
        return;
    }
    QDBusMessage message = QDBusMessage::createMethodCall(current, MPRIS_PATH, PLAYER_IFACE, method);
    QDBusConnection::sessionBus().asyncCall(message, 2000);
}

void MediaController::pauseOthers(const QString& except)
{
    for(auto player = players.cbegin(); player != players.cend(); ++player)
    {
        if(player.key() != except && player->status == QLatin1String("Playing"))
        {
            QDBusConnection::sessionBus().asyncCall(
                QDBusMessage::createMethodCall(player.key(), MPRIS_PATH, PLAYER_IFACE, QStringLiteral("Pause")), 2000);
        }
    }
}

void MediaController::previous() { call(QStringLiteral("Previous")); }
void MediaController::playPause() { call(QStringLiteral("PlayPause")); }
void MediaController::next() { call(QStringLiteral("Next")); }

QString MediaController::identity() const
{
    return hasPlayer() ? players.value(current).identity : QString();
}

QString MediaController::iconName() const
{
    return hasPlayer() ? players.value(current).desktopEntry : QString();
}

QString MediaController::track() const
{
    return hasPlayer() ? players.value(current).metadata.value(QStringLiteral("xesam:title")).toString() : QString();
}

QString MediaController::artist() const
{
    if(!hasPlayer())
    {
        return QString();
    }
    const QVariant artists = players.value(current).metadata.value(QStringLiteral("xesam:artist"));
    QStringList names;
    for(const QVariant& name : artists.toList())
    {
        names.append(name.toString());
    }
    return names.isEmpty() ? artists.toString() : names.join(QStringLiteral(", "));
}

QString MediaController::artUrl() const
{
    return hasPlayer() ? players.value(current).metadata.value(QStringLiteral("mpris:artUrl")).toString() : QString();
}

bool MediaController::playing() const
{
    return hasPlayer() && players.value(current).status == QLatin1String("Playing");
}

bool MediaController::canGoNext() const { return hasPlayer() && players.value(current).canGoNext; }
bool MediaController::canGoPrevious() const { return hasPlayer() && players.value(current).canGoPrevious; }
bool MediaController::canPlay() const { return hasPlayer() && players.value(current).canPlay; }
bool MediaController::canPause() const { return hasPlayer() && players.value(current).canPause; }
bool MediaController::canSeek() const { return hasPlayer() && players.value(current).canSeek && length() > 0; }

double MediaController::position() const
{
    return hasPlayer() ? players.value(current).position / 1000.0 : 0.0;
}

double MediaController::length() const
{
    return hasPlayer()
        ? players.value(current).metadata.value(QStringLiteral("mpris:length")).toLongLong() / 1000.0 : 0.0;
}
