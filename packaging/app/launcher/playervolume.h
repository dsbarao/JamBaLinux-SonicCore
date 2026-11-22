/*---------------------------------------------------------*\
| playervolume.h                                            |
|                                                           |
|   The volume of the player shown in "Tocando agora" (a    |
|   browser's video, Spotify, the radio): its own audio     |
|   streams in PipeWire, like the desktop mixer's volume    |
|   per application. Browsers do not take a volume through  |
|   MPRIS, so the streams are found by process: the player's|
|   D-Bus process or one it started (browsers play from a   |
|   child process), else by the application's name.         |
\*---------------------------------------------------------*/

#pragma once

#include <QList>
#include <QObject>
#include <QProcess>
#include <QStringList>
#include <QTimer>

class PlayerVolume : public QObject
{
    Q_OBJECT
    /* The player's process (MediaController.pid); 0 when unknown. */
    Q_PROPERTY(qint64 pid READ pid WRITE setPid NOTIFY targetChanged)
    /* Names it goes by (desktop entry, identity), for the name match. */
    Q_PROPERTY(QStringList names READ names WRITE setNames NOTIFY targetChanged)
    /* It has audio streams now. */
    Q_PROPERTY(bool available READ available NOTIFY changed)
    Q_PROPERTY(int percent READ percent NOTIFY changed)
    Q_PROPERTY(bool muted READ muted NOTIFY changed)

public:
    explicit PlayerVolume(QObject* parent = nullptr);
    ~PlayerVolume() override;

    qint64 pid() const { return playerPid; }
    void setPid(qint64 value);
    QStringList names() const { return playerNames; }
    void setNames(const QStringList& value);
    bool available() const { return !matched.isEmpty(); }
    int percent() const { return volumePercent; }
    bool muted() const { return isMuted; }

    /* 0..100 on every stream of the player; unmutes them. */
    Q_INVOKABLE void setPercent(int percent);
    Q_INVOKABLE void setMuted(bool muted);

    struct Stream
    {
        int          index = -1;
        qint64       pid = 0;
        QString      binary;
        QString      app;
        int          percent = 0;
        bool         muted = false;
        /* Its process's parents, nearest first. */
        QList<qint64> ancestors;
    };
    /* `pactl -f json list sink-inputs`, completed by `pactl -f json list
       clients`: a native PipeWire stream names its process only on its
       client (ancestors left empty). */
    static QList<Stream> parse(const QByteArray& json, const QByteArray& clientsJson = QByteArray());
    /* Which streams belong to the player: by process first, by name when
       no process matches. Returns positions in `streams`. */
    static QList<int> match(const QList<Stream>& streams, qint64 pid, const QStringList& names);

Q_SIGNALS:
    void targetChanged();
    void changed();

private:
    void startSubscriber();
    void refresh();
    void apply(const QList<Stream>& streams);
    void run(const QStringList& pactlArguments);
    void runNext();

    QProcess  subscriber;
    QProcess* reader = nullptr;
    QProcess* writer = nullptr;
    QTimer    refreshSoon;
    bool      refreshAgain = false;
    QList<QStringList> pendingWrites;

    qint64      playerPid = 0;
    QStringList playerNames;
    QList<int>  matched;          /* sink-input indexes */
    int         volumePercent = 0;
    bool        isMuted = false;
};
