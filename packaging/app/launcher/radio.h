/*---------------------------------------------------------*\
| radio.h                                                   |
|                                                           |
|   Internet radio for the "Rádio" section: the station     |
|   directory of Radio Browser (radio-browser.info, a free, |
|   community-run catalog with an open API), searched by    |
|   style (tags) and region, keeping only stations that     |
|   answer, best voted first. Nothing is sent until the     |
|   radio is used; station "clicks" are not reported.       |
\*---------------------------------------------------------*/

#pragma once

#include <QByteArray>
#include <QList>
#include <QNetworkAccessManager>
#include <QObject>
#include <QStringList>
#include <QVariantList>

class QNetworkReply;

class RadioDirectory : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QVariantList stations READ stations NOTIFY stationsChanged)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)

public:
    explicit RadioDirectory(QObject* parent = nullptr);

    QVariantList stations() const { return found; }
    bool busy() const { return pending > 0; }
    QString error() const { return message; }

    /* Stations with any of `tags`; `countryCode` ("BR") or empty for the
       whole world. Replaces `stations` when the answers arrive. */
    Q_INVOKABLE void search(const QStringList& tags, const QString& countryCode);

    /* The answers of Radio Browser's /json/stations/search, one per tag,
       merged: every station once (by id and by name), only those that
       answered the last check with a stream address and a listenable bit
       rate, at most `limit`. Stations devoted to the style come first (the
       share of their tags that are `styleTags`, the style in their name),
       then the popular ones (listened to now, then votes). Each is
       { uuid, name, url, favicon, country, codec, bitrate, tags }. */
    static QVariantList rank(const QList<QByteArray>& answers, const QStringList& styleTags, int limit);

Q_SIGNALS:
    void stationsChanged();
    void busyChanged();
    void errorChanged();

private:
    void discoverServer();
    void request(const QString& tag, const QString& countryCode, int generation);
    void finish(int generation);
    void setError(const QString& text);

    QNetworkAccessManager network;
    QString server;               /* one Radio Browser mirror, picked once */
    bool    discovering = false;
    QList<QPair<QStringList, QString>> waiting;  /* searches waiting for a server */
    QList<QByteArray> answers;
    QStringList styleTags;        /* the tags of the search running */
    int     pending = 0;
    int     generation = 0;       /* a newer search wins over an older one */
    QVariantList found;
    QString message;
};

/* The song a station announces (Shoutcast/Icecast "StreamTitle"), read
   with a short request of its own: the player does not pass it on. Only
   the first metadata block is read (about one second of audio), then the
   connection is closed. */
class RadioTitle : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString title READ title NOTIFY titleChanged)

public:
    explicit RadioTitle(QObject* parent = nullptr);

    QString title() const { return current; }

    Q_INVOKABLE void read(const QString& url);
    Q_INVOKABLE void clear();

    /* "StreamTitle='Artist - Song';StreamUrl='';" → "Artist - Song". */
    static QString parse(const QByteArray& block);

Q_SIGNALS:
    void titleChanged();

private:
    void setTitle(const QString& value);

    QNetworkAccessManager network;
    QNetworkReply* reply = nullptr;
    QString current;
};

