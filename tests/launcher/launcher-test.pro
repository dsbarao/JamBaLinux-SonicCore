# Logic tests for the app launcher (packaging/app/launcher), built and run by
# tests/launcher_logic.rs. No window, player or PipeWire is needed.
TEMPLATE = app
TARGET = launcher-test
QT += core qml dbus
CONFIG += c++17 console warn_on
CONFIG -= app_bundle
QMAKE_CXXFLAGS += -Werror
LAUNCHER = $$PWD/../../packaging/app/launcher
INCLUDEPATH += $$LAUNCHER
HEADERS += $$LAUNCHER/dbushelpers.h $$LAUNCHER/executable.h $$LAUNCHER/media.h $$LAUNCHER/volume.h
SOURCES += launcher.test.cpp $$LAUNCHER/dbushelpers.cpp $$LAUNCHER/executable.cpp \
    $$LAUNCHER/media.cpp $$LAUNCHER/volume.cpp
