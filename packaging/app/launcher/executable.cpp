#include "executable.h"

#include <QFile>

#include <csignal>
#include <sys/prctl.h>

Executable::Executable(QObject* parent)
    : QObject(parent)
{
}

Executable::~Executable()
{
    for(QProcess* process : std::as_const(running))
    {
        process->disconnect(this);
        process->kill();
        process->waitForFinished(500);
        delete process;
    }
}

QStringList Executable::shellArguments(const QString& command, QString* program)
{
    if(QFile::exists(QStringLiteral("/.flatpak-info")))
    {
        *program = QStringLiteral("flatpak-spawn");
        return { QStringLiteral("--host"), QStringLiteral("--watch-bus"), QStringLiteral("/bin/sh"),
                 QStringLiteral("-c"), command };
    }
    *program = QStringLiteral("/bin/sh");
    return { QStringLiteral("-c"), command };
}

void Executable::endWithApp(QProcess* process)
{
    process->setChildProcessModifier([]() {
        ::prctl(PR_SET_PDEATHSIG, SIGTERM);
    });
}

void Executable::connectSource(const QString& command)
{
    if(command.isEmpty() || running.contains(command))
    {
        return;
    }
    auto* process = new QProcess(this);
    running.insert(command, process);
    Q_EMIT connectedSourcesChanged();

    connect(process, &QProcess::finished, this, [this, command, process](int code, QProcess::ExitStatus status) {
        QVariantMap data;
        data.insert(QStringLiteral("exit code"), status == QProcess::NormalExit ? code : -1);
        data.insert(QStringLiteral("exit status"), status == QProcess::NormalExit ? 0 : 1);
        data.insert(QStringLiteral("stdout"), QString::fromUtf8(process->readAllStandardOutput()));
        data.insert(QStringLiteral("stderr"), QString::fromUtf8(process->readAllStandardError()));
        running.remove(command);
        process->deleteLater();
        Q_EMIT connectedSourcesChanged();
        Q_EMIT newData(command, data);
    });
    connect(process, &QProcess::errorOccurred, this, [this, command, process](QProcess::ProcessError error) {
        if(error != QProcess::FailedToStart)
        {
            return;
        }
        QVariantMap data;
        data.insert(QStringLiteral("exit code"), 127);
        data.insert(QStringLiteral("stdout"), QString());
        data.insert(QStringLiteral("stderr"), process->errorString());
        running.remove(command);
        process->deleteLater();
        Q_EMIT connectedSourcesChanged();
        Q_EMIT newData(command, data);
    });

    QString program;
    const QStringList arguments = shellArguments(command, &program);
    endWithApp(process);
    process->start(program, arguments);
}

void Executable::disconnectSource(const QString&)
{
}

QStringList Executable::connectedSources() const
{
    return running.keys();
}
