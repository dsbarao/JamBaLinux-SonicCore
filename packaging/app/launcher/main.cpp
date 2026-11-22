/*---------------------------------------------------------*\
| main.cpp                                                  |
|                                                           |
|   JamBaLinux Sonic Core app. Opens the QML interface with |
|   the helpers it needs built in (commands, D-Bus, media,  |
|   volume), so it runs on any desktop: Plasma, GNOME,      |
|   COSMIC, Cinnamon… On Plasma it uses Plasma's own        |
|   controls, elsewhere Qt's, picked at start-up.           |
|                                                           |
|   Usage: soniccore-app [--tray] [--controls plasma|qt]    |
\*---------------------------------------------------------*/

#include "appinfo.h"
#include "dbushelpers.h"
#include "executable.h"
#include "media.h"
#include "volume.h"

#include <QApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QIcon>
#include <QLocalServer>
#include <QLocalSocket>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QSystemTrayIcon>
#include <QTimer>
#include <QUrl>

#include <QSocketNotifier>

#include <csignal>
#include <sys/socket.h>
#include <unistd.h>

/* SIGTERM/SIGINT (logout, `kill`) end the app like "Sair": the event loop
   returns and every helper process is stopped on the way out. */
static int signalPipe[2] = { -1, -1 };

static void onTerminate(int)
{
    const char byte = 1;
    [[maybe_unused]] const ssize_t written = ::write(signalPipe[1], &byte, 1);
}

static const char* URI = "org.jambalinux.soniccore";

/* The folder holding main.qml: next to the binary once installed, or the
   one named by SONICCORE_APP_DIR (running from the source tree). */
static QString findAppDir()
{
    const QString overridden = qEnvironmentVariable("SONICCORE_APP_DIR");
    if(!overridden.isEmpty())
    {
        return QDir(overridden).absolutePath();
    }
    return QCoreApplication::applicationDirPath();
}

static bool moduleAvailable(const QStringList& importPaths, const QString& module)
{
    const QString relative = QString(module).replace(QLatin1Char('.'), QLatin1Char('/')) + QStringLiteral("/qmldir");
    for(const QString& path : importPaths)
    {
        if(QFile::exists(path + QLatin1Char('/') + relative))
        {
            return true;
        }
    }
    return false;
}

static QString argumentValue(const QStringList& arguments, const QString& name)
{
    const int index = arguments.indexOf(name);
    return index >= 0 && index + 1 < arguments.size() ? arguments.at(index + 1) : QString();
}

int main(int argc, char** argv)
{
    /* The effects read the colors the OpenRGB plugin publishes in a small
       local file. */
    qputenv("QML_XHR_ALLOW_FILE_READ", "1");

    QApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("jambalinux-soniccore"));
    /* The window's app id: every desktop matches it with the launcher's
       .desktop file, for its icon and name in the task bar (in a Flatpak,
       the Flatpak's own id and entry). */
    const QString flatpakId = qEnvironmentVariable("FLATPAK_ID");
    app.setDesktopFileName(flatpakId.isEmpty() ? QStringLiteral("jambalinux-soniccore") : flatpakId);

    const QStringList arguments = app.arguments();
    const bool trayStart = arguments.contains(QStringLiteral("--tray"));
    const QString appDir = findAppDir();
    if(!QFile::exists(appDir + QStringLiteral("/main.qml")))
    {
        qCritical("main.qml not found in %s (set SONICCORE_APP_DIR)", qPrintable(appDir));
        return 1;
    }
    app.setWindowIcon(QIcon(appDir + QStringLiteral("/jambalinux-soniccore.svg")));

    /* One copy per user: a second launch shows the running window. */
    const QString socketName = QStringLiteral("jambalinux-soniccore-app-%1").arg(getuid());
    {
        QLocalSocket probe;
        probe.connectToServer(socketName);
        if(probe.waitForConnected(500))
        {
            probe.write(trayStart ? "tray\n" : "show\n");
            probe.waitForBytesWritten(500);
            probe.disconnectFromServer();
            return 0;
        }
    }

    AppInfo info;
    info.desktop = qEnvironmentVariable("XDG_CURRENT_DESKTOP");
    info.plasma = info.desktop.split(QLatin1Char(':')).contains(QStringLiteral("KDE"), Qt::CaseInsensitive);
    info.flatpak = QFile::exists(QStringLiteral("/.flatpak-info"));
    info.setTrayAvailable(QSystemTrayIcon::isSystemTrayAvailable());

    QLocalServer instanceServer;
    QLocalServer::removeServer(socketName);
    instanceServer.listen(socketName);
    QObject::connect(&instanceServer, &QLocalServer::newConnection, &instanceServer, [&]() {
        while(QLocalSocket* client = instanceServer.nextPendingConnection())
        {
            QObject::connect(client, &QLocalSocket::readyRead, client, [&info, client]() {
                if(client->readAll().startsWith("show"))
                {
                    Q_EMIT info.activateRequested();
                }
                client->disconnectFromServer();
            });
            QObject::connect(client, &QLocalSocket::disconnected, client, &QObject::deleteLater);
        }
    });

    if(::socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, signalPipe) == 0)
    {
        auto* notifier = new QSocketNotifier(signalPipe[0], QSocketNotifier::Read, &app);
        QObject::connect(notifier, &QSocketNotifier::activated, &app, &QCoreApplication::quit);
        std::signal(SIGTERM, onTerminate);
        std::signal(SIGINT, onTerminate);
    }

    QQmlApplicationEngine engine;
    const QStringList importPaths = engine.importPathList();

    /* Plasma's controls on Plasma (the look the app was designed with),
       Qt's everywhere else; --controls or SONICCORE_CONTROLS overrides. */
    QString controls = argumentValue(arguments, QStringLiteral("--controls"));
    if(controls.isEmpty())
    {
        controls = qEnvironmentVariable("SONICCORE_CONTROLS");
    }
    if(controls != QLatin1String("plasma") && controls != QLatin1String("qt"))
    {
        controls = info.plasma && moduleAvailable(importPaths, QStringLiteral("org.kde.plasma.components"))
            ? QStringLiteral("plasma") : QStringLiteral("qt");
    }
    info.controls = controls;
    engine.addImportPath(appDir + QStringLiteral("/controls/") + controls);

    /* Qt's controls look native with KDE's desktop style when it is
       installed (any desktop), and fall back to Fusion. */
    if(qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE"))
    {
        QQuickStyle::setStyle(moduleAvailable(importPaths, QStringLiteral("org.kde.desktop"))
                                  ? QStringLiteral("org.kde.desktop") : QStringLiteral("Fusion"));
    }

    qmlRegisterType<Executable>(URI, 1, 0, "Executable");
    qmlRegisterType<SignalWatcher>(URI, 1, 0, "SignalWatcher");
    qmlRegisterType<ServiceWatcher>(URI, 1, 0, "ServiceWatcher");
    qmlRegisterType<MediaController>(URI, 1, 0, "MediaController");
    qmlRegisterType<OutputVolume>(URI, 1, 0, "OutputVolume");
    qmlRegisterUncreatableType<DBusReply>(URI, 1, 0, "DBusReply", QStringLiteral("returned by SessionBus.asyncCall"));
    SessionBus sessionBus;
    qmlRegisterSingletonInstance(URI, 1, 0, "SessionBus", &sessionBus);
    qmlRegisterSingletonInstance(URI, 1, 0, "App", &info);

    /* A tray may appear a little after login (the panel starts late). */
    QTimer trayCheck;
    QObject::connect(&trayCheck, &QTimer::timeout, &info, [&info]() {
        info.setTrayAvailable(QSystemTrayIcon::isSystemTrayAvailable());
    });
    trayCheck.start(3000);

    engine.load(QUrl::fromLocalFile(appDir + QStringLiteral("/main.qml")));
    if(engine.rootObjects().isEmpty())
    {
        return 1;
    }
    return app.exec();
}
