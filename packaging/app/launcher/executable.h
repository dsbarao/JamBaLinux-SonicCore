/*---------------------------------------------------------*\
| executable.h                                              |
|                                                           |
|   Runs a shell command line and reports its exit code,    |
|   stdout and stderr, with the same shape as Plasma's      |
|   "executable" data engine (connectSource / newData), so  |
|   the interface needs no Plasma library on any desktop.   |
|   Inside a Flatpak the command runs on the host through   |
|   flatpak-spawn, where the soniccore CLI is installed.    |
\*---------------------------------------------------------*/

#pragma once

#include <QHash>
#include <QObject>
#include <QProcess>
#include <QString>
#include <QStringList>
#include <QVariantMap>

class Executable : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QStringList connectedSources READ connectedSources NOTIFY connectedSourcesChanged)

public:
    explicit Executable(QObject* parent = nullptr);
    ~Executable() override;

    /* Starts `command` (a /bin/sh command line). A command already running
       is not started twice: like a data engine source, it answers once. */
    Q_INVOKABLE void connectSource(const QString& command);
    /* Accepted for compatibility; a finished command is already gone. */
    Q_INVOKABLE void disconnectSource(const QString& command);

    QStringList connectedSources() const;

    /* The argument list that runs `command` here or on the Flatpak host. */
    static QStringList shellArguments(const QString& command, QString* program);
    /* The command ends with the app, even when the app is killed: no
       `pactl subscribe` is left running (on the Flatpak host, flatpak-spawn
       --watch-bus ends it once flatpak-spawn is gone). */
    static void endWithApp(QProcess* process);

Q_SIGNALS:
    void newData(const QString& sourceName, const QVariantMap& data);
    void connectedSourcesChanged();

private:
    QHash<QString, QProcess*> running;
};
