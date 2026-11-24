QT += core gui qml quick quickcontrols2

# The tray icon is QSystemTrayIcon (src/tray.cpp), the one implementation that
# is native on both platforms: a notification-area icon on Windows and a
# StatusNotifierItem on Linux, which is what Quickshell's tray shows.
#
# Qt.labs.platform was the obvious alternative and does not work here: its
# SystemTrayIcon backend exists only on Linux, so on Windows `available` stays
# false and the icon never appears.
QT += widgets

# Nothing in the app speaks D-Bus directly; it is listed only because the theme
# watcher used to. Keeping it off Windows means the same .pro builds on every
# platform.
unix: QT += dbus

CONFIG += c++17 release
TARGET = taildrop-gui
TEMPLATE = app

# An .rc that only sets the icon: Windows reads the executable's own icon for
# the taskbar, Alt-Tab and Explorer, and ignores a window icon that is set at
# runtime for those. Without it the app is a blank window in the task switcher.
win32: RC_ICONS = src/icons/taildrop.ico

# Shadow builds (CI) and in-tree builds (gui/bin/build) both work.
DESTDIR = $$OUT_PWD

HEADERS += \
    src/appicon.h \
    src/backend.h \
    src/singleinstance.h \
    src/theme.h \
    src/tray.h

SOURCES += \
    src/main.cpp \
    src/backend.cpp \
    src/singleinstance.cpp \
    src/theme.cpp \
    src/tray.cpp

RESOURCES += src/resources.qrc
