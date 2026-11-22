# OpenRGB plugin for the JBL Quantum 810 Wireless through JamBaLinux Sonic Core.
#
# Build against the OpenRGB source tree matching the installed OpenRGB
# (plugin API 5, OpenRGB 1.1):
#   qmake6 OPENRGB_DIR=/path/to/OpenRGB && make
# tools/install-openrgb-plugin.sh does this for you.

isEmpty(OPENRGB_DIR) {
    error("Set OPENRGB_DIR to an OpenRGB 1.1 source tree: qmake6 OPENRGB_DIR=/path/to/OpenRGB")
}
!exists($$OPENRGB_DIR/OpenRGBPluginInterface.h) {
    error("$$OPENRGB_DIR does not contain OpenRGBPluginInterface.h")
}

TEMPLATE  = lib
TARGET    = JamBaLinuxSonicCorePlugin
CONFIG   += plugin c++17 release
QT       += core gui widgets

INCLUDEPATH += \
    $$OPENRGB_DIR \
    $$OPENRGB_DIR/RGBController \
    $$OPENRGB_DIR/dependencies/json

HEADERS += \
    JamBaLinuxPlugin.h \
    LightingPlan.h \
    SoniccoreClient.h

SOURCES += \
    JamBaLinuxPlugin.cpp \
    SoniccoreClient.cpp

DISTFILES += JamBaLinuxPlugin.json
