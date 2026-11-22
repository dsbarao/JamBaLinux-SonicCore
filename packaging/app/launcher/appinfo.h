/*---------------------------------------------------------*\
| appinfo.h                                                 |
|                                                           |
|   What the interface needs to know about where it runs:   |
|   the desktop, whether a tray exists, which set of        |
|   controls it was given (Plasma's or Qt's), and a second  |
|   launch asking the running window to show itself, and    |
|   the compositor's blur behind the window.                |
\*---------------------------------------------------------*/

#pragma once

#include <QObject>
#include <QString>
#include <QWindow>

#include "windoweffects.h"

class AppInfo : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString desktop MEMBER desktop CONSTANT)
    Q_PROPERTY(bool plasma MEMBER plasma CONSTANT)
    Q_PROPERTY(QString controls MEMBER controls CONSTANT)
    Q_PROPERTY(bool trayAvailable READ trayAvailable NOTIFY trayAvailableChanged)
    Q_PROPERTY(bool flatpak MEMBER flatpak CONSTANT)

public:
    explicit AppInfo(QObject* parent = nullptr)
        : QObject(parent)
    {
    }

    bool trayAvailable() const { return tray; }
    void setTrayAvailable(bool available)
    {
        if(available != tray)
        {
            tray = available;
            Q_EMIT trayAvailableChanged();
        }
    }

    /* "Vidro": the compositor blurs what is behind the window (KWin). */
    Q_INVOKABLE bool blurAvailable() const { return WindowEffects::blurAvailable(); }
    Q_INVOKABLE bool setBlurBehind(QObject* window, bool enabled)
    {
        return WindowEffects::setBlurBehind(qobject_cast<QWindow*>(window), enabled);
    }

    QString desktop;
    bool    plasma = false;
    QString controls;
    bool    flatpak = false;

Q_SIGNALS:
    void trayAvailableChanged();
    /* Another launch (the menu, a shortcut) asked to see the window. */
    void activateRequested();

private:
    bool tray = false;
};
