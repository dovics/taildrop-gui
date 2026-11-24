QT += core gui qml quick quickcontrols2

# Nothing in the app speaks D-Bus; it is listed only because the theme watcher
# used to. Keeping it off Windows means the same .pro builds on every platform.
unix: QT += dbus

CONFIG += c++17 release
TARGET = taildrop-gui
TEMPLATE = app

# Shadow builds (CI) and in-tree builds (gui/bin/build) both work.
DESTDIR = $$OUT_PWD

HEADERS += \
    src/backend.h \
    src/theme.h

SOURCES += \
    src/main.cpp \
    src/backend.cpp \
    src/theme.cpp

RESOURCES += src/resources.qrc
