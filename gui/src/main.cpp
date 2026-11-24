// Taildrop — send files over your tailnet by dropping them on a device.

#include <QGuiApplication>
#include <QIcon>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QUrl>

#include <cstdlib>
#include <cstring>

#include "backend.h"
#include "theme.h"

// Launching from the app launcher is not the same as launching from a shell.
//
// Omarchy's launcher runs `uwsm-app -- gtk-launch ...`, which starts the app in
// a systemd unit whose environment is not the graphical session's: without
// WAYLAND_DISPLAY / XDG_RUNTIME_DIR, Qt aborts with "XDG_RUNTIME_DIR is invalid
// or not set" before a window exists, and from the outside that is a window
// that flashes and vanishes. Read the values the session does have and fill in
// only the ones that are missing, so a deliberately overridden variable wins.
static void adoptSessionEnvironment() {
    static const char *keys[] = {"XDG_RUNTIME_DIR", "WAYLAND_DISPLAY",
                                 "DISPLAY", "XDG_SESSION_TYPE"};
    bool missing = false;
    for (const char *key : keys) {
        const char *value = std::getenv(key);
        if (!value || !*value) {
            missing = true;
            break;
        }
    }
    if (!missing)
        return;

    // `systemctl --user show-environment` is what the login session exported;
    // it is authoritative and needs no guessing about display numbers.
    FILE *pipe = popen("systemctl --user show-environment 2>/dev/null", "r");
    if (!pipe)
        return;

    char line[512];
    while (std::fgets(line, sizeof line, pipe)) {
        char *eq = std::strchr(line, '=');
        if (!eq)
            continue;
        *eq = '\0';
        // Trim the trailing newline the shell hands us.
        char *value = eq + 1;
        value[std::strcspn(value, "\r\n")] = '\0';
        if (!*value)
            continue;

        bool wanted = false;
        for (const char *key : keys)
            if (std::strcmp(line, key) == 0)
                wanted = true;
        if (!wanted)
            continue;
        // Do not clobber an explicit override.
        const char *existing = std::getenv(line);
        if (existing && *existing)
            continue;
        setenv(line, value, 1);
    }
    pclose(pipe);
}

int main(int argc, char *argv[]) {
    // Must happen before QGuiApplication: Qt reads the platform variables in
    // its constructor and aborts if the runtime directory is absent.
    adoptSessionEnvironment();

    QGuiApplication app(argc, argv);
    app.setApplicationName("taildrop-gui");
    app.setApplicationDisplayName("Taildrop");
    app.setApplicationVersion("0.1.0");

    // Matches the window to taildrop-gui.desktop, so the compositor (Wayland app_id)
    // and the launcher pick up the installed icon.
    app.setDesktopFileName("taildrop-gui");
    app.setWindowIcon(QIcon::fromTheme("taildrop-gui"));

    QQuickStyle::setStyle("Material");

    Theme theme;
    Backend backend;

    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty("theme", &theme);
    engine.rootContext()->setContextProperty("backend", &backend);
    engine.load(QUrl("qrc:/Main.qml"));
    if (engine.rootObjects().isEmpty()) {
        // A GUI that dies without a word is undiagnosable from the launcher,
        // where there is no terminal to read.
        qCritical("Taildrop: the window could not be created (QML failed to load).");
        return 1;
    }

    return app.exec();
}
