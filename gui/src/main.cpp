// Taildrop — send files over your tailnet by dropping them on a device.

#include <QGuiApplication>
#include <QIcon>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QUrl>

#include <QCommandLineParser>

#include <cstdlib>
#include <cstring>

#include "backend.h"
#include "singleinstance.h"
#include "theme.h"

// Launching from the app launcher is not the same as launching from a shell.
//
// Omarchy's launcher runs `uwsm-app -- gtk-launch ...`, which starts the app in
// a systemd unit whose environment is not the graphical session's: without
// WAYLAND_DISPLAY / XDG_RUNTIME_DIR, Qt aborts with "XDG_RUNTIME_DIR is invalid
// or not set" before a window exists, and from the outside that is a window
// that flashes and vanishes. Read the values the session does have and fill in
// only the ones that are missing, so a deliberately overridden variable wins.
//
// Linux only, and the guard is load-bearing: popen() and setenv() are POSIX and
// MSVC has neither, so compiling this on Windows fails outright. Windows needs
// no equivalent — its launcher hands the app the normal desktop environment.
static void adoptSessionEnvironment() {
#if defined(Q_OS_LINUX)
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
#endif
}

int main(int argc, char *argv[]) {
    // Must happen before QGuiApplication: Qt reads the platform variables in
    // its constructor and aborts if the runtime directory is absent.
    adoptSessionEnvironment();

    QGuiApplication app(argc, argv);
    app.setApplicationName("taildrop-gui");
    // The tray and a second launch both talk to this name; changing it would
    // silently allow two receivers that fight over the same port.
    app.setOrganizationName("taildrop");
    app.setApplicationDisplayName("Taildrop");
    app.setApplicationVersion("0.1.0");

    // Matches the window to taildrop-gui.desktop, so the compositor (Wayland app_id)
    // and the launcher pick up the installed icon.
    app.setDesktopFileName("taildrop-gui");
    app.setWindowIcon(QIcon::fromTheme("taildrop-gui"));

    QQuickStyle::setStyle("Material");

    // A receiver that must survive its window is started by the session with
    // --background: listening from the first second, no window in the way.
    QCommandLineParser parser;
    parser.setApplicationDescription("Send files over your tailnet.");
    parser.addHelpOption();
    parser.addVersionOption();
    const QCommandLineOption background(
        {"b", "background"},
        "Start listening without showing the window (used at login).");
    parser.addOption(background);
    const QCommandLineOption show(
        {"s", "show"}, "Bring the running window forward, then exit.");
    parser.addOption(show);
    parser.process(app);
    const bool startHidden = parser.isSet(background);
    // Launching the app again is a request to see it: the dock icon, the
    // launcher, and `taildrop-gui --show` all mean the same thing.
    const bool wantWindow = !startHidden || parser.isSet(show);

    SingleInstance instance("taildrop-gui");
    if (!instance.claim(wantWindow))
        return 0;

    Theme theme;
    Backend backend;

    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty("theme", &theme);
    engine.rootContext()->setContextProperty("instance", &instance);
    // Read by the window before it is created, so a background start never
    // paints a window first and hides it after.
    engine.rootContext()->setContextProperty("startHiddenAtLaunch", startHidden);
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
