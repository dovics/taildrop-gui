#pragma once

#include <QObject>
#include <QString>

class QAction;
class QMenu;
class QSystemTrayIcon;

// The tray icon, in C++ rather than QML.
//
// Qt.labs.platform's SystemTrayIcon looks like the convenient choice — one QML
// type, no C++ — but its platform backend is implemented only on Linux, so on
// Windows `available` is false and the icon silently never appears. That is the
// worst kind of failure for a background app: the window closes into nothing
// and there is no way back. QSystemTrayIcon is the real cross-platform one:
// native on Windows, and a StatusNotifierItem on Linux, which is what the
// Omarchy tray shows.
class Tray : public QObject {
    Q_OBJECT

public:
    explicit Tray(QObject *parent = nullptr);
    ~Tray() override;

    // False on a desktop with no tray at all. The window uses it to decide
    // whether hiding is safe: with no icon there would be no way back.
    bool available() const;

    // What the icon says about the app right now. Kept in one place so the
    // tooltip, the menu's first row and the window title never disagree.
    Q_INVOKABLE void updateState(bool receiving, int peers);

    // The menu's Open/Hide row is about the window, and only the window knows
    // whether it is showing.
    Q_INVOKABLE void setWindowVisible(bool visible);

    // A hidden receiver that never speaks is indistinguishable from a dead
    // one, so an arrival announces itself.
    Q_INVOKABLE void announceArrival(const QString &fileName);

signals:
    // The icon or its menu asked for something only the window can do, because
    // showing a window owned by QML is QML's business.
    void toggleRequested();
    void rescanRequested();
    void quitRequested();

private:
    void refresh();

    QSystemTrayIcon *m_icon = nullptr;
    QMenu *m_menu = nullptr;
    QAction *m_status = nullptr;
    QAction *m_toggle = nullptr;
    QAction *m_rescan = nullptr;
    QAction *m_quit = nullptr;

    bool m_receiving = false;
    bool m_visible = false;
    int m_peers = 0;
};
