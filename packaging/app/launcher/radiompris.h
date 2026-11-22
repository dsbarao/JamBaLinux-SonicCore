/*---------------------------------------------------------*\
| radiompris.h                                              |
|                                                           |
|   The radio as a media player of the desktop (MPRIS):     |
|   while it is on, "Tocando agora", the keyboard's media   |
|   keys and the desktop's media controls see it like any   |
|   player, and their play/pause, next and previous reach   |
|   the interface as signals (next = next station).         |
\*---------------------------------------------------------*/

#pragma once

#include <QDBusAbstractAdaptor>
#include <QDBusObjectPath>
#include <QObject>
#include <QStringList>
#include <QVariantMap>

class RadioMpris : public QObject
{
    Q_OBJECT
    /* Published on the session bus while true. */
    Q_PROPERTY(bool active READ active WRITE setActive NOTIFY activeChanged)
    /* "Playing", "Paused" or "Stopped". */
    Q_PROPERTY(QString playbackStatus READ playbackStatus WRITE setPlaybackStatus NOTIFY playbackStatusChanged)
    /* The song the station announces, or the station's name. */
    Q_PROPERTY(QString title READ title WRITE setTitle NOTIFY metadataChanged)
    Q_PROPERTY(QString station READ station WRITE setStation NOTIFY metadataChanged)
    Q_PROPERTY(QString artUrl READ artUrl WRITE setArtUrl NOTIFY metadataChanged)
    /* Its bus name (a copy started with SONICCORE_INSTANCE has its own). */
    Q_PROPERTY(QString service READ service CONSTANT)

public:
    QString service() const { return serviceName; }

    explicit RadioMpris(QObject* parent = nullptr);
    ~RadioMpris() override;

    bool active() const { return published; }
    void setActive(bool active);
    QString playbackStatus() const { return status; }
    void setPlaybackStatus(const QString& value);
    QString title() const { return songTitle; }
    void setTitle(const QString& value);
    QString station() const { return stationName; }
    void setStation(const QString& value);
    QString artUrl() const { return art; }
    void setArtUrl(const QString& value);

    QVariantMap metadata() const;

Q_SIGNALS:
    void activeChanged();
    void playbackStatusChanged();
    void metadataChanged();
    /* Requests from the desktop (media keys, panels, "Tocando agora"). */
    void playRequested();
    void pauseRequested();
    void playPauseRequested();
    void stopRequested();
    void nextRequested();
    void previousRequested();
    void raiseRequested();

private:
    void announce(const QVariantMap& changed);

    QString  serviceName;
    QObject* exported = nullptr;   /* holds the two adaptors */
    bool     published = false;
    QString  status = QStringLiteral("Stopped");
    QString  songTitle;
    QString  stationName;
    QString  art;
    quint32  track = 0;            /* a new track id for each station */
};

/* org.mpris.MediaPlayer2 */
class RadioRootAdaptor : public QDBusAbstractAdaptor
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.mpris.MediaPlayer2")
    Q_PROPERTY(bool CanQuit READ canQuit)
    Q_PROPERTY(bool CanRaise READ canRaise)
    Q_PROPERTY(bool HasTrackList READ hasTrackList)
    Q_PROPERTY(QString Identity READ identity)
    Q_PROPERTY(QString DesktopEntry READ desktopEntry)
    Q_PROPERTY(QStringList SupportedUriSchemes READ none)
    Q_PROPERTY(QStringList SupportedMimeTypes READ none)

public:
    RadioRootAdaptor(QObject* exported, RadioMpris* radio);

    bool canQuit() const { return false; }
    bool canRaise() const { return true; }
    bool hasTrackList() const { return false; }
    QString identity() const;
    QString desktopEntry() const;
    QStringList none() const { return {}; }

public Q_SLOTS:
    void Raise();
    void Quit() {}

private:
    RadioMpris* radio;
};

/* org.mpris.MediaPlayer2.Player */
class RadioPlayerAdaptor : public QDBusAbstractAdaptor
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.mpris.MediaPlayer2.Player")
    Q_PROPERTY(QString PlaybackStatus READ playbackStatus)
    Q_PROPERTY(double Rate READ one)
    Q_PROPERTY(double MinimumRate READ one)
    Q_PROPERTY(double MaximumRate READ one)
    Q_PROPERTY(double Volume READ one)
    Q_PROPERTY(QVariantMap Metadata READ metadata)
    Q_PROPERTY(qlonglong Position READ position)
    Q_PROPERTY(bool CanGoNext READ yes)
    Q_PROPERTY(bool CanGoPrevious READ yes)
    Q_PROPERTY(bool CanPlay READ yes)
    Q_PROPERTY(bool CanPause READ yes)
    Q_PROPERTY(bool CanSeek READ no)
    Q_PROPERTY(bool CanControl READ yes)

public:
    RadioPlayerAdaptor(QObject* exported, RadioMpris* radio);

    QString playbackStatus() const;
    QVariantMap metadata() const;
    double one() const { return 1.0; }
    qlonglong position() const { return 0; }
    bool yes() const { return true; }
    bool no() const { return false; }

public Q_SLOTS:
    void Next();
    void Previous();
    void Pause();
    void PlayPause();
    void Stop();
    void Play();
    void Seek(qlonglong) {}
    void SetPosition(const QDBusObjectPath&, qlonglong) {}
    void OpenUri(const QString&) {}

Q_SIGNALS:
    void Seeked(qlonglong position);

private:
    RadioMpris* radio;
};
