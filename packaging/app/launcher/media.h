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
#include <QVariantMap>

class MediaController : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool hasPlayer READ hasPlayer NOTIFY changed)
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

public:
    explicit MediaController(QObject* parent = nullptr);

    bool hasPlayer() const { return players.contains(current); }
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

    Q_INVOKABLE void previous();
    Q_INVOKABLE void playPause();
    Q_INVOKABLE void next();

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
        qint64      activeSince = 0; /* when it last started playing */
    };

    /* Which of `players` to show: the one that started playing last; if
       none plays, `current` while it exists, else the last one active. */
    static QString choose(const QHash<QString, Player>& players, const QString& current);

Q_SIGNALS:
    void changed();

private Q_SLOTS:
    void nameOwnerChanged(const QString& name, const QString& oldOwner, const QString& newOwner);
    void propertiesChanged(const QDBusMessage& message);

private:
    void addPlayer(const QString& service, const QString& owner);
    void fetch(const QString& service, const QString& iface);
    void apply(Player& player, const QVariantMap& properties);
    void call(const QString& method);
    void update();

    QHash<QString, Player> players;
    QString current;
};
