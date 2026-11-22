/*---------------------------------------------------------*\
| volume.h                                                  |
|                                                           |
|   The computer's output volume (the default sink), read   |
|   and set with pactl, which PipeWire provides on every    |
|   desktop. `pactl subscribe` reports changes made         |
|   elsewhere (keyboard keys, the panel) as they happen.    |
\*---------------------------------------------------------*/

#pragma once

#include <QObject>
#include <QProcess>
#include <QString>
#include <QTimer>

class OutputVolume : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool available READ available NOTIFY changed)
    Q_PROPERTY(QString description READ description NOTIFY changed)
    Q_PROPERTY(int percent READ percent NOTIFY changed)
    Q_PROPERTY(bool muted READ muted NOTIFY changed)

public:
    explicit OutputVolume(QObject* parent = nullptr);
    ~OutputVolume() override;

    bool available() const { return !sinkName.isEmpty(); }
    QString description() const { return sinkDescription; }
    int percent() const { return volumePercent; }
    bool muted() const { return isMuted; }

    /* 0..100; unmutes, like moving the panel's slider. */
    Q_INVOKABLE void setPercent(int percent);
    Q_INVOKABLE void setMuted(bool muted);

    struct Reading
    {
        QString name;
        QString description;
        int     percent = 0;
        bool    muted = false;
    };
    /* The default sink from `pactl get-default-sink` followed by
       `pactl -f json list sinks`; an empty name when it is not listed. */
    static Reading parse(const QString& defaultName, const QByteArray& sinksJson);

Q_SIGNALS:
    void changed();

private:
    void startSubscriber();
    void refresh();
    void run(const QStringList& pactlArguments);
    void runNext();

    QProcess  subscriber;
    QProcess* reader = nullptr;
    QProcess* writer = nullptr;
    QTimer    refreshSoon;
    bool      refreshAgain = false;
    QList<QStringList> pendingWrites;

    QString sinkName;
    QString sinkDescription;
    int     volumePercent = 0;
    bool    isMuted = false;
};
