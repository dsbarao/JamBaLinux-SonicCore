/*---------------------------------------------------------*\
| dbushelpers.h                                             |
|                                                           |
|   The few session-bus tools the interface uses, in plain  |
|   Qt D-Bus: an asynchronous method call, a signal watcher |
|   and a service watcher. They replace the Plasma-only     |
|   org.kde.plasma.workspace.dbus module.                   |
\*---------------------------------------------------------*/

#pragma once

#include <QDBusMessage>
#include <QDBusServiceWatcher>
#include <QObject>
#include <QQmlParserStatus>
#include <QVariant>
#include <QVariantList>
#include <QVariantMap>

/* D-Bus values as plain QML values: arrays, structs and dictionaries
   become lists and maps, recursively. */
QVariant plainDBusValue(const QVariant& value);

class DBusReply : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool isError READ isError NOTIFY finished)
    Q_PROPERTY(QVariantList values READ values NOTIFY finished)
    Q_PROPERTY(QString error READ error NOTIFY finished)

public:
    explicit DBusReply(QObject* parent = nullptr);

    bool isError() const { return failed; }
    QVariantList values() const { return replyValues; }
    QString error() const { return message; }

    void complete(const QDBusMessage& reply);

Q_SIGNALS:
    void finished();

private:
    bool         failed = false;
    QVariantList replyValues;
    QString      message;
};

class SessionBus : public QObject
{
    Q_OBJECT

public:
    explicit SessionBus(QObject* parent = nullptr);

    /* { service, path, iface, member, arguments } like Plasma's
       SessionBus.asyncCall; the reply emits finished(). */
    Q_INVOKABLE DBusReply* asyncCall(const QVariantMap& call);
};

class SignalWatcher : public QObject, public QQmlParserStatus
{
    Q_OBJECT
    Q_INTERFACES(QQmlParserStatus)
    Q_PROPERTY(QString service MEMBER service NOTIFY changed)
    Q_PROPERTY(QString path MEMBER path NOTIFY changed)
    Q_PROPERTY(QString iface MEMBER iface NOTIFY changed)
    Q_PROPERTY(QString signalName MEMBER signalName NOTIFY changed)
    Q_PROPERTY(bool enabled MEMBER enabled NOTIFY changed)

public:
    explicit SignalWatcher(QObject* parent = nullptr);

    void classBegin() override {}
    void componentComplete() override;

Q_SIGNALS:
    void changed();
    /* One emitted D-Bus signal, with its arguments as plain values. */
    void received(const QString& name, const QVariantList& arguments);

private Q_SLOTS:
    void deliver(const QDBusMessage& message);

private:
    void reconnect();

    QString service;
    QString path;
    QString iface;
    QString signalName;
    bool    enabled = true;
    bool    complete = false;
    bool    connected = false;
    QString connectedService, connectedPath, connectedIface, connectedSignal;
};

class ServiceWatcher : public QObject, public QQmlParserStatus
{
    Q_OBJECT
    Q_INTERFACES(QQmlParserStatus)
    Q_PROPERTY(QString watchedService READ watchedService WRITE setWatchedService NOTIFY watchedServiceChanged)
    Q_PROPERTY(bool registered READ registered NOTIFY registeredChanged)

public:
    explicit ServiceWatcher(QObject* parent = nullptr);

    void classBegin() override {}
    void componentComplete() override;

    QString watchedService() const { return name; }
    void setWatchedService(const QString& service);
    bool registered() const { return isRegistered; }

Q_SIGNALS:
    void watchedServiceChanged();
    void registeredChanged();

private:
    void setRegistered(bool value);

    QDBusServiceWatcher watcher;
    QString name;
    bool    isRegistered = false;
};
