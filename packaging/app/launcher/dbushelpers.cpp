#include "dbushelpers.h"

#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusObjectPath>
#include <QDBusPendingCallWatcher>
#include <QDBusSignature>
#include <QDBusVariant>
#include <QQmlEngine>

static QVariant demarshal(const QDBusArgument& argument)
{
    switch(argument.currentType())
    {
    case QDBusArgument::BasicType:
    case QDBusArgument::VariantType:
        return plainDBusValue(argument.asVariant());
    case QDBusArgument::ArrayType:
    {
        QVariantList list;
        argument.beginArray();
        while(!argument.atEnd())
        {
            list.append(demarshal(argument));
        }
        argument.endArray();
        return list;
    }
    case QDBusArgument::StructureType:
    {
        QVariantList fields;
        argument.beginStructure();
        while(!argument.atEnd())
        {
            fields.append(demarshal(argument));
        }
        argument.endStructure();
        return fields;
    }
    case QDBusArgument::MapType:
    {
        QVariantMap map;
        argument.beginMap();
        while(!argument.atEnd())
        {
            argument.beginMapEntry();
            const QVariant key = demarshal(argument);
            const QVariant value = demarshal(argument);
            argument.endMapEntry();
            map.insert(key.toString(), value);
        }
        argument.endMap();
        return map;
    }
    default:
        return QVariant();
    }
}

QVariant plainDBusValue(const QVariant& value)
{
    if(value.canConvert<QDBusArgument>() && value.metaType() == QMetaType::fromType<QDBusArgument>())
    {
        return demarshal(value.value<QDBusArgument>());
    }
    if(value.metaType() == QMetaType::fromType<QDBusVariant>())
    {
        return plainDBusValue(value.value<QDBusVariant>().variant());
    }
    if(value.metaType() == QMetaType::fromType<QDBusObjectPath>())
    {
        return value.value<QDBusObjectPath>().path();
    }
    if(value.metaType() == QMetaType::fromType<QDBusSignature>())
    {
        return value.value<QDBusSignature>().signature();
    }
    return value;
}

DBusReply::DBusReply(QObject* parent)
    : QObject(parent)
{
}

void DBusReply::complete(const QDBusMessage& reply)
{
    failed = reply.type() != QDBusMessage::ReplyMessage;
    message = failed ? reply.errorMessage() : QString();
    replyValues.clear();
    for(const QVariant& value : reply.arguments())
    {
        replyValues.append(plainDBusValue(value));
    }
    Q_EMIT finished();
}

SessionBus::SessionBus(QObject* parent)
    : QObject(parent)
{
}

DBusReply* SessionBus::asyncCall(const QVariantMap& call)
{
    /* Owned here, not by the JavaScript garbage collector, which could
       otherwise collect it before the answer arrives. */
    auto* reply = new DBusReply(this);
    QQmlEngine::setObjectOwnership(reply, QQmlEngine::CppOwnership);
    QDBusMessage message = QDBusMessage::createMethodCall(
        call.value(QStringLiteral("service")).toString(),
        call.value(QStringLiteral("path")).toString(),
        call.value(QStringLiteral("iface")).toString(),
        call.value(QStringLiteral("member")).toString());
    message.setArguments(call.value(QStringLiteral("arguments")).toList());
    auto* watcher = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(message, 2000), reply);
    QObject::connect(watcher, &QDBusPendingCallWatcher::finished, reply, [reply, watcher]() {
        reply->complete(watcher->reply());
        watcher->deleteLater();
        /* The caller has had its finished() signal; nothing else holds it. */
        reply->deleteLater();
    });
    return reply;
}

SignalWatcher::SignalWatcher(QObject* parent)
    : QObject(parent)
{
    connect(this, &SignalWatcher::changed, this, &SignalWatcher::reconnect);
}

void SignalWatcher::componentComplete()
{
    complete = true;
    reconnect();
}

void SignalWatcher::reconnect()
{
    if(!complete)
    {
        return;
    }
    QDBusConnection bus = QDBusConnection::sessionBus();
    if(connected)
    {
        bus.disconnect(connectedService, connectedPath, connectedIface, connectedSignal,
                       this, SLOT(deliver(QDBusMessage)));
        connected = false;
    }
    if(!enabled || path.isEmpty() || iface.isEmpty() || signalName.isEmpty())
    {
        return;
    }
    connected = bus.connect(service, path, iface, signalName, this, SLOT(deliver(QDBusMessage)));
    connectedService = service;
    connectedPath = path;
    connectedIface = iface;
    connectedSignal = signalName;
}

void SignalWatcher::deliver(const QDBusMessage& message)
{
    QVariantList arguments;
    for(const QVariant& value : message.arguments())
    {
        arguments.append(plainDBusValue(value));
    }
    Q_EMIT received(message.member(), arguments);
}

ServiceWatcher::ServiceWatcher(QObject* parent)
    : QObject(parent)
{
    watcher.setConnection(QDBusConnection::sessionBus());
    watcher.setWatchMode(QDBusServiceWatcher::WatchForOwnerChange);
    connect(&watcher, &QDBusServiceWatcher::serviceRegistered, this, [this]() { setRegistered(true); });
    connect(&watcher, &QDBusServiceWatcher::serviceUnregistered, this, [this]() { setRegistered(false); });
}

void ServiceWatcher::componentComplete()
{
    if(name.isEmpty())
    {
        return;
    }
    QDBusConnectionInterface* bus = QDBusConnection::sessionBus().interface();
    setRegistered(bus && bus->isServiceRegistered(name).value());
}

void ServiceWatcher::setWatchedService(const QString& service)
{
    if(service == name)
    {
        return;
    }
    name = service;
    watcher.setWatchedServices({ service });
    Q_EMIT watchedServiceChanged();
    componentComplete();
}

void ServiceWatcher::setRegistered(bool value)
{
    if(value == isRegistered)
    {
        return;
    }
    isRegistered = value;
    Q_EMIT registeredChanged();
}
