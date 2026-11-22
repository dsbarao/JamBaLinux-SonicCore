/*---------------------------------------------------------*\
| media.h                                                   |
|                                                           |
|   "Tocando agora": the media player to show and control,  |
|   read straight from the MPRIS D-Bus interface every      |
|   Linux player publishes (browsers, Spotify, VLC, mpv…),  |
|   so it works on any desktop, not only Plasma.            |
\*---------------------------------------------------------*/

#pragma once

#include <QDBusMessage>
#include <QHash>
#include <QObject>
#include <QTimer>
#include <QVariantMap>

class MediaController : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool hasPlayer READ hasPlayer NOTIFY changed)
    /* The bus name of the player shown (the app's own radio included). */
    Q_PROPERTY(QString service READ service NOTIFY changed)
    Q_PROPERTY(QString identity READ identity NOTIFY changed)
    Q_PROPERTY(QString iconName READ iconName NOTIFY changed)
    Q_PROPERTY(QString track READ track NOTIFY changed)
    Q_PROPERTY(QString artist READ artist NOTIFY changed)
    Q_PROPERTY(QString artUrl READ artUrl NOTIFY changed)
    Q_PROPERTY(bool playing READ playing NOTIFY changed)
    Q_PROPERTY(bool canGoNext READ canGoNext NOTIFY changed)
    Q_PROPERTY(bool canGoPrevious READ canGoPrevious NOTIFY changed)
    Q_PROPERTY(bool canPlay READ canPlay NOTIFY changed)
    Q_PROPERTY(bool canPause READ canPause NOTIFY changed)
    /* Where the track is and how long it is, in milliseconds (0 when the
       player does not say); canSeek when it can be moved. */
    Q_PROPERTY(double position READ position NOTIFY positionChanged)
    Q_PROPERTY(double length READ length NOTIFY changed)
    Q_PROPERTY(bool canSeek READ canSeek NOTIFY changed)
    /* The player's process (its D-Bus connection's), 0 until known: what
       its audio streams are found by. */
    Q_PROPERTY(qint64 pid READ pid NOTIFY changed)

public:
    explicit MediaController(QObject* parent = nullptr);

    bool hasPlayer() const { return players.contains(current); }
    QString service() const { return hasPlayer() ? current : QString(); }
    QString identity() const;
    QString iconName() const;
    QString track() const;
    QString artist() const;
    QString artUrl() const;
    bool playing() const;
    bool canGoNext() const;
    bool canGoPrevious() const;
    bool canPlay() const;
    bool canPause() const;
    double position() const;
    double length() const;
    bool canSeek() const;
    qint64 pid() const { return hasPlayer() ? players.value(current).pid : 0; }

    Q_INVOKABLE void previous();
    Q_INVOKABLE void playPause();
    Q_INVOKABLE void next();
    /* Pauses every player that plays, except `except` (a bus name). */
    Q_INVOKABLE void pauseOthers(const QString& except);
    /* Moves the track to `milliseconds` from its start. */
    Q_INVOKABLE void setPosition(double milliseconds);

    struct Player
    {
        QString     owner;           /* unique bus name the signals come from */
        QString     identity;
        QString     desktopEntry;
        QVariantMap metadata;
        QString     status;
        bool        canGoNext = false;
        bool        canGoPrevious = false;
        bool        canPlay = false;
        bool        canPause = false;
        bool        canSeek = false;
        qint64      position = 0;    /* microseconds, as MPRIS counts */
        qint64      activeSince = 0; /* when it last started playing */
        qint64      pid = 0;
    };

    /* Which of `players` to show: the one that started playing last; if
       none plays, `current` while it exists, else the last one active. */
    static QString choose(const QHash<QString, Player>& players, const QString& current);

Q_SIGNALS:
    void changed();
    void positionChanged();

private Q_SLOTS:
    void nameOwnerChanged(const QString& name, const QString& oldOwner, const QString& newOwner);
    void propertiesChanged(const QDBusMessage& message);
    void seeked(const QDBusMessage& message);

private:
    void addPlayer(const QString& service, const QString& owner);
    void fetch(const QString& service, const QString& iface);
    void apply(Player& player, const QVariantMap& properties);
    void call(const QString& method);
    void update();
    void fetchPosition();

    QHash<QString, Player> players;
    QString current;
    /* The position is not announced while it advances: read it once a
       second while a track plays (a tiny D-Bus call). */
    QTimer positionPoll;
    QString shownTrack;
};
