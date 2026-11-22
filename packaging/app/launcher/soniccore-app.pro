# JamBaLinux Sonic Core app launcher (Qt 6). Built by tools/install-user.sh:
#   qmake6 packaging/app/launcher/soniccore-app.pro && make
TEMPLATE = app
TARGET = soniccore-app
QT += core gui widgets qml quick quickcontrols2 dbus network
CONFIG += c++17 release warn_on


HEADERS += appinfo.h dbushelpers.h executable.h imagepalette.h media.h playervolume.h radio.h radiompris.h volume.h windoweffects.h
SOURCES += main.cpp dbushelpers.cpp executable.cpp imagepalette.cpp media.cpp playervolume.cpp radio.cpp radiompris.cpp volume.cpp windoweffects.cpp
# The compositor's blur (windoweffects.cpp) is loaded at run time.
LIBS += -ldl
