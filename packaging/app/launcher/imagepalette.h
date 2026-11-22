/*---------------------------------------------------------*\
| imagepalette.h                                            |
|                                                           |
|   The main colors of a picture, for the "Papel de parede" |
|   and "Capa do que toca" themes of "Luzes do PC": the     |
|   desktop wallpaper and the cover (or station logo) of    |
|   what is playing. Local files are read directly; web     |
|   covers are fetched once, small and with a time limit.   |
\*---------------------------------------------------------*/

#pragma once

#include <QImage>
#include <QNetworkAccessManager>
#include <QObject>
#include <QPointer>
#include <QStringList>
#include <QUrl>

class QNetworkReply;

class ImagePalette : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QUrl source READ source WRITE setSource NOTIFY sourceChanged)
    /* Up to four "#rrggbb", the most present (vivid) hues first; empty
       while there is no picture. */
    Q_PROPERTY(QStringList colors READ colors NOTIFY colorsChanged)

public:
    explicit ImagePalette(QObject* parent = nullptr);

    QUrl source() const { return url; }
    void setSource(const QUrl& value);
    QStringList colors() const { return found; }

    /* The palette of a decoded picture. */
    static QStringList extract(const QImage& image);

Q_SIGNALS:
    void sourceChanged();
    void colorsChanged();

private:
    void setColors(const QStringList& value);

    QNetworkAccessManager network;
    QPointer<QNetworkReply> reply;
    QUrl url;
    QStringList found;
};
