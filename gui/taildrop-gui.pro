QT += core gui qml quick quickcontrols2

# Qt.labs.platform's SystemTrayIcon is the one cross-platform way to a tray
# icon: on Linux it becomes a StatusNotifierItem, which Quickshell's tray picks
# up, and on Windows a notification-area icon. The module links QtWidgets
# underneath, so it has to be built in rather than merely imported.
QT += widgets

# Nothing in the app speaks D-Bus directly; it is listed only because the theme
# watcher used to. Keeping it off Windows means the same .pro builds on every
# platform.
unix: QT += dbus

CONFIG += c++17 release
TARGET = taildrop-gui
TEMPLATE = app

# Shadow builds (CI) and in-tree builds (gui/bin/build) both work.
DESTDIR = $$OUT_PWD

HEADERS += \
    src/backend.h \
    src/singleinstance.h \
    src/theme.h

SOURCES += \
    src/main.cpp \
    src/backend.cpp \
    src/singleinstance.cpp \
    src/theme.cpp

RESOURCES += src/resources.qrc
