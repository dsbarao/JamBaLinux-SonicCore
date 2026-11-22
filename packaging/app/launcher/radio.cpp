#include "radio.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QRandomGenerator>
#include <QSet>
#include <QUrlQuery>

#include <algorithm>
#include <cmath>
#include <memory>

/* Radio Browser asks every client to say who it is. */
static const QByteArray USER_AGENT =
    "JamBaLinuxSonicCore/0.5 (+https://github.com/dsbarao/JamBaLinux-SonicCore)";
/* Round-robin name of every mirror; also answers queries itself. */
static const QString ALL_SERVERS = QStringLiteral("all.api.radio-browser.info");
/* Below this a stream sounds poor; 0 means "not said" and is kept. */
static constexpr int MINIMUM_BITRATE = 48;

static QNetworkRequest apiRequest(const QUrl& url)
{
    QNetworkRequest request(url);
    request.setHeader(QNetworkRequest::UserAgentHeader, USER_AGENT);
    request.setRawHeader("Accept", "application/json");
    request.setTransferTimeout(10000);
    return request;
}

RadioDirectory::RadioDirectory(QObject* parent)
    : QObject(parent)
{
}

void RadioDirectory::setError(const QString& text)
{
    if(text != message)
    {
        message = text;
        Q_EMIT errorChanged();
    }
}

void RadioDirectory::discoverServer()
{
    if(discovering)
    {
        return;
    }
    discovering = true;
    QNetworkReply* reply = network.get(apiRequest(QUrl(QStringLiteral("https://%1/json/servers").arg(ALL_SERVERS))));
    connect(reply, &QNetworkReply::finished, this, [this, reply]() {
        reply->deleteLater();
        discovering = false;
        QStringList names;
        if(reply->error() == QNetworkReply::NoError)
        {
            for(const QJsonValue& entry : QJsonDocument::fromJson(reply->readAll()).array())
            {
                const QString name = entry.toObject().value(QStringLiteral("name")).toString();
                if(name.endsWith(QLatin1String(".radio-browser.info")) && !names.contains(name))
                {
                    names.append(name);
                }
            }
        }
        /* One mirror for this session, as Radio Browser recommends; the
           round-robin name when the list cannot be read. */
        server = names.isEmpty() ? ALL_SERVERS
                                 : names.at(QRandomGenerator::global()->bounded(names.size()));
        const auto queued = waiting;
        waiting.clear();
        for(const auto& search : queued)
        {
            this->search(search.first, search.second);
        }
    });
}

void RadioDirectory::search(const QStringList& tags, const QString& countryCode)
{
    generation++;
    if(server.isEmpty())
    {
        waiting = { { tags, countryCode } };
        if(pending == 0)
        {
            pending = 1;   /* busy while the mirror is looked up */
            Q_EMIT busyChanged();
        }
        discoverServer();
        return;
    }
    answers.clear();
    const bool wasBusy = busy();
    pending = 0;
    const QStringList asked = tags.mid(0, 4);
    styleTags = tags;
    for(const QString& tag : asked)
    {
        pending++;
        request(tag, countryCode, generation);
    }
    if(busy() != wasBusy)
    {
        Q_EMIT busyChanged();
    }
    if(asked.isEmpty())
    {
        finish(generation);
    }
}

void RadioDirectory::request(const QString& tag, const QString& countryCode, int searchGeneration)
{
    QUrl url(QStringLiteral("https://%1/json/stations/search").arg(server));
    QUrlQuery query;
    /* The exact tag ("reggae" must not bring "reggaeton"), most listened
       to now first. */
    query.addQueryItem(QStringLiteral("tag"), tag);
    query.addQueryItem(QStringLiteral("tagExact"), QStringLiteral("true"));
    query.addQueryItem(QStringLiteral("hidebroken"), QStringLiteral("true"));
    query.addQueryItem(QStringLiteral("order"), QStringLiteral("clickcount"));
    query.addQueryItem(QStringLiteral("reverse"), QStringLiteral("true"));
    query.addQueryItem(QStringLiteral("limit"), QStringLiteral("80"));
    if(!countryCode.isEmpty())
    {
        query.addQueryItem(QStringLiteral("countrycode"), countryCode);
    }
    url.setQuery(query);
    QNetworkReply* reply = network.get(apiRequest(url));
    connect(reply, &QNetworkReply::finished, this, [this, reply, searchGeneration]() {
        reply->deleteLater();
        if(searchGeneration != generation)
        {
            return;
        }
        if(reply->error() == QNetworkReply::NoError)
        {
            answers.append(reply->readAll());
        }
        pending--;
        if(pending == 0)
        {
            finish(searchGeneration);
        }
    });
}

void RadioDirectory::finish(int searchGeneration)
{
    if(searchGeneration != generation)
    {
        return;
    }
    const bool reached = !answers.isEmpty();
    found = rank(answers, styleTags, 60);
    answers.clear();
    Q_EMIT stationsChanged();
    Q_EMIT busyChanged();
    if(!reached)
    {
        /* Maybe that mirror is down: look for another next time. */
        server.clear();
        setError(QStringLiteral("Não foi possível falar com o catálogo de rádios. Verifique a internet."));
    }
    else
    {
        setError(found.isEmpty() ? QStringLiteral("Nenhuma estação desse estilo no ar agora.") : QString());
    }
}

QVariantList RadioDirectory::rank(const QList<QByteArray>& replies, const QStringList& styleTags, int limit)
{
    struct Station
    {
        QVariantMap fields;
        double score = 0;
    };
    QSet<QString> wanted;
    for(const QString& tag : styleTags)
    {
        wanted.insert(tag.trimmed().toLower());
    }
    QList<Station> stations;
    QSet<QString> seen;
    QSet<QString> seenNames;
    for(const QByteArray& reply : replies)
    {
        for(const QJsonValue& value : QJsonDocument::fromJson(reply).array())
        {
            const QJsonObject station = value.toObject();
            const QString uuid = station.value(QStringLiteral("stationuuid")).toString();
            QString url = station.value(QStringLiteral("url_resolved")).toString().trimmed();
            if(url.isEmpty())
            {
                url = station.value(QStringLiteral("url")).toString().trimmed();
            }
            const int bitrate = station.value(QStringLiteral("bitrate")).toInt();
            const QString name = station.value(QStringLiteral("name")).toString().simplified();
            const QString nameKey = name.toLower();
            /* A name that is a list of keywords ("TOP 100 … Reggaeton, Urban,
               HipHop, Club & Party Radio …") is spam, not a station name. */
            if(name.size() > 64)
            {
                continue;
            }
            if(uuid.isEmpty() || seen.contains(uuid) || seenNames.contains(nameKey) || url.isEmpty() || name.isEmpty()
               || station.value(QStringLiteral("lastcheckok")).toInt() != 1
               || (bitrate > 0 && bitrate < MINIMUM_BITRATE)
               || !(url.startsWith(QLatin1String("http://")) || url.startsWith(QLatin1String("https://"))))
            {
                continue;
            }
            seen.insert(uuid);
            seenNames.insert(nameKey);
            Station entry;
            const double votes = qMax<qint64>(0, station.value(QStringLiteral("votes")).toVariant().toLongLong());
            const double clicks = qMax<qint64>(0, station.value(QStringLiteral("clickcount")).toVariant().toLongLong());
            /* Devotion to the style: a reggae station, not a station that
               plays reggae among forty other things. */
            const QStringList tags = station.value(QStringLiteral("tags")).toString().toLower().split(QLatin1Char(','), Qt::SkipEmptyParts);
            int matching = 0;
            for(const QString& tag : tags)
            {
                matching += wanted.contains(tag.trimmed()) ? 1 : 0;
            }
            const double focus = tags.isEmpty() ? 0.0 : double(matching) / std::sqrt(double(tags.size()));
            bool named = false;
            for(const QString& tag : std::as_const(wanted))
            {
                named = named || (tag.size() >= 3 && nameKey.contains(tag));
            }
            entry.score = 2.0 * focus + (named ? 1.0 : 0.0)
                + 0.6 * std::log1p(clicks) + 0.35 * std::log1p(votes)
                + (bitrate >= 96 ? 0.3 : 0.0)
                + (station.value(QStringLiteral("favicon")).toString().isEmpty() ? 0.0 : 0.2);
            entry.fields = {
                { QStringLiteral("uuid"), uuid },
                { QStringLiteral("name"), name },
                { QStringLiteral("url"), url },
                { QStringLiteral("favicon"), station.value(QStringLiteral("favicon")).toString().trimmed() },
                { QStringLiteral("country"), station.value(QStringLiteral("countrycode")).toString() },
                { QStringLiteral("codec"), station.value(QStringLiteral("codec")).toString() },
                { QStringLiteral("bitrate"), bitrate },
                { QStringLiteral("tags"), station.value(QStringLiteral("tags")).toString() },
            };
            stations.append(entry);
        }
    }
    std::stable_sort(stations.begin(), stations.end(), [](const Station& a, const Station& b) {
        return a.score > b.score;
    });
    QVariantList ranked;
    for(const Station& station : std::as_const(stations))
    {
        if(ranked.size() >= limit)
        {
            break;
        }
        ranked.append(station.fields);
    }
    return ranked;
}

RadioTitle::RadioTitle(QObject* parent)
    : QObject(parent)
{
}

void RadioTitle::setTitle(const QString& value)
{
    if(value != current)
    {
        current = value;
        Q_EMIT titleChanged();
    }
}

void RadioTitle::clear()
{
    if(reply)
    {
        reply->abort();
    }
    setTitle(QString());
}

QString RadioTitle::parse(const QByteArray& block)
{
    const QByteArray marker = "StreamTitle='";
    const int start = block.indexOf(marker);
    if(start < 0)
    {
        return QString();
    }
    const int from = start + marker.size();
    int end = block.indexOf("';", from);
    if(end < 0)
    {
        end = block.lastIndexOf('\'');
    }
    if(end < from)
    {
        return QString();
    }
    const QByteArray raw = block.mid(from, end - from);
    /* Most stations send UTF-8; older ones Latin-1. */
    QString title = QString::fromUtf8(raw);
    if(title.contains(QChar::ReplacementCharacter))
    {
        title = QString::fromLatin1(raw);
    }
    return title.simplified();
}

void RadioTitle::read(const QString& url)
{
    if(reply)
    {
        reply->abort();
    }
    QNetworkRequest request{QUrl(url)};
    request.setHeader(QNetworkRequest::UserAgentHeader, USER_AGENT);
    request.setRawHeader("Icy-MetaData", "1");
    request.setTransferTimeout(8000);
    QNetworkReply* answer = network.get(request);
    reply = answer;
    auto buffer = std::make_shared<QByteArray>();
    connect(answer, &QNetworkReply::readyRead, this, [this, answer, buffer]() {
        buffer->append(answer->readAll());
        const int metaint = answer->rawHeader("icy-metaint").toInt();
        if(metaint <= 0 || metaint > 512 * 1024)
        {
            answer->abort();   /* no titles from this station */
            return;
        }
        if(buffer->size() <= metaint)
        {
            return;
        }
        const int length = static_cast<unsigned char>(buffer->at(metaint)) * 16;
        if(buffer->size() < metaint + 1 + length)
        {
            return;
        }
        const QString title = parse(buffer->mid(metaint + 1, length));
        /* An empty block means "same song as before". */
        if(!title.isEmpty())
        {
            setTitle(title);
        }
        answer->abort();
    });
    connect(answer, &QNetworkReply::finished, this, [this, answer]() {
        answer->deleteLater();
        if(reply == answer)
        {
            reply = nullptr;
        }
    });
}

