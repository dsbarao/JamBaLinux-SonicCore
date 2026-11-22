#include "imagepalette.h"

#include <QColor>
#include <QNetworkReply>
#include <QNetworkRequest>

#include <algorithm>
#include <array>
#include <cmath>

/* Covers are small; anything this large is not one. */
static constexpr qint64 MAX_DOWNLOAD = 4 * 1024 * 1024;
static constexpr int HUE_BINS = 24;

ImagePalette::ImagePalette(QObject* parent)
    : QObject(parent)
{
}

void ImagePalette::setColors(const QStringList& value)
{
    if(value != found)
    {
        found = value;
        Q_EMIT colorsChanged();
    }
}

void ImagePalette::setSource(const QUrl& value)
{
    if(value == url)
    {
        return;
    }
    url = value;
    Q_EMIT sourceChanged();
    if(reply)
    {
        reply->abort();
    }
    if(url.isLocalFile())
    {
        QImage image(url.toLocalFile());
        setColors(extract(image));
        return;
    }
    const QString scheme = url.scheme();
    if(scheme != QLatin1String("http") && scheme != QLatin1String("https"))
    {
        setColors({});
        return;
    }
    QNetworkRequest request(url);
    request.setHeader(QNetworkRequest::UserAgentHeader,
                      QByteArray("JamBaLinuxSonicCore/0.5 (+https://github.com/dsbarao/JamBaLinux-SonicCore)"));
    request.setTransferTimeout(8000);
    QNetworkReply* answer = network.get(request);
    reply = answer;
    connect(answer, &QNetworkReply::downloadProgress, answer, [answer](qint64 received, qint64) {
        if(received > MAX_DOWNLOAD)
        {
            answer->abort();
        }
    });
    connect(answer, &QNetworkReply::finished, this, [this, answer]() {
        answer->deleteLater();
        if(answer != reply)
        {
            return;
        }
        setColors(answer->error() == QNetworkReply::NoError
                      ? extract(QImage::fromData(answer->readAll()))
                      : QStringList());
    });
}

QStringList ImagePalette::extract(const QImage& picture)
{
    if(picture.isNull())
    {
        return {};
    }
    const QImage image = picture.convertToFormat(QImage::Format_ARGB32)
                             .scaled(64, 64, Qt::IgnoreAspectRatio, Qt::FastTransformation);
    struct Bin
    {
        double weight = 0;
        double r = 0;
        double g = 0;
        double b = 0;
    };
    std::array<Bin, HUE_BINS> bins{};
    double grayR = 0;
    double grayG = 0;
    double grayB = 0;
    double grayWeight = 0;
    for(int y = 0; y < image.height(); y++)
    {
        const QRgb* line = reinterpret_cast<const QRgb*>(image.constScanLine(y));
        for(int x = 0; x < image.width(); x++)
        {
            if(qAlpha(line[x]) < 128)
            {
                continue;
            }
            const QColor color(line[x]);
            const double saturation = color.hsvSaturationF();
            const double value = color.valueF();
            if(saturation < 0.25 || value < 0.25)
            {
                grayR += color.redF();
                grayG += color.greenF();
                grayB += color.blueF();
                grayWeight += 1;
                continue;
            }
            /* Vivid pixels count more: they are what lights can show. */
            const double weight = saturation * value * saturation;
            const int bin = std::clamp(int(color.hsvHueF() * HUE_BINS), 0, HUE_BINS - 1);
            bins[bin].weight += weight;
            bins[bin].r += color.redF() * weight;
            bins[bin].g += color.greenF() * weight;
            bins[bin].b += color.blueF() * weight;
        }
    }
    std::array<int, HUE_BINS> order{};
    for(int index = 0; index < HUE_BINS; index++)
    {
        order[index] = index;
    }
    std::sort(order.begin(), order.end(), [&bins](int a, int b) { return bins[a].weight > bins[b].weight; });
    const double strongest = bins[order[0]].weight;
    QStringList colors;
    QList<int> taken;
    for(int index : order)
    {
        const Bin& bin = bins[index];
        if(colors.size() >= 4 || bin.weight <= 0 || bin.weight < strongest * 0.06)
        {
            break;
        }
        /* Neighbouring hues are the same color for a light. */
        const bool near = std::any_of(taken.begin(), taken.end(), [index](int other) {
            const int distance = std::abs(index - other);
            return std::min(distance, HUE_BINS - distance) < 2;
        });
        if(near)
        {
            continue;
        }
        taken.append(index);
        colors.append(QColor::fromRgbF(float(bin.r / bin.weight), float(bin.g / bin.weight),
                                       float(bin.b / bin.weight))
                          .name());
    }
    if(colors.isEmpty() && grayWeight > 0)
    {
        /* A black-and-white picture: its average tone. */
        colors.append(QColor::fromRgbF(float(grayR / grayWeight), float(grayG / grayWeight),
                                       float(grayB / grayWeight))
                          .name());
    }
    return colors;
}
