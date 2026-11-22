#include "media.h"
#include "dbushelpers.h"

#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusPendingCallWatcher>
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
