#include "tray.h"

#include <QAction>
#include <QApplication>
#include <QIcon>
#include <QMenu>
#include <QSystemTrayIcon>

#include "appicon.h"

Tray::Tray(QObject *parent) : QObject(parent) {
    // On X11 a tray that is created too early can miss the notification area
    // that is still starting up, so ask again once the event loop has settled.
    // QSystemTrayIcon keeps checking on its own, but available() is what the
    // window asks, and that must be true before the user closes anything.
    m_icon = new QSystemTrayIcon(taildropIcon(), this);
    m_icon->setToolTip(QStringLiteral("Taildrop"));

    // The menu takes no parent on purpose: parenting it to the icon makes it a
    // child widget of a hidden widget tree on some platforms, and a QMenu owned
    // by nothing is one this class deletes in its destructor.
    m_menu = new QMenu();

    m_status = m_menu->addAction(QString());
    m_status->setEnabled(false);
    m_menu->addSeparator();

    m_toggle = m_menu->addAction(QString());
    connect(m_toggle, &QAction::triggered, this, &Tray::toggleRequested);

    m_rescan = m_menu->addAction(tr("Rescan devices"));
    connect(m_rescan, &QAction::triggered, this, &Tray::rescanRequested);

    m_menu->addSeparator();
    m_quit = m_menu->addAction(tr("Quit"));
    connect(m_quit, &QAction::triggered, this, &Tray::quitRequested);

    m_icon->setContextMenu(m_menu);

    // A left click is the shortest path back to the window, on both platforms:
    // Windows only sends Trigger for a left click, and a StatusNotifier host
    // sends Trigger or DoubleClick depending on its settings.
    connect(m_icon, &QSystemTrayIcon::activated, this,
            [this](QSystemTrayIcon::ActivationReason reason) {
                if (reason == QSystemTrayIcon::Trigger ||
                    reason == QSystemTrayIcon::DoubleClick)
                    emit toggleRequested();
            });

    m_icon->setVisible(true);
    refresh();
}

Tray::~Tray() {
    // The menu is deliberately parentless so it is not owned by the icon's
    // widget tree; it has to be deleted here.
    delete m_menu;
}

bool Tray::available() const {
    return m_icon && m_icon->isVisible() &&
           QSystemTrayIcon::isSystemTrayAvailable();
}

void Tray::updateState(bool receiving, int peers) {
    m_receiving = receiving;
    m_peers = peers;
    refresh();
}

void Tray::setWindowVisible(bool visible) {
    if (visible == m_visible)
        return;
    m_visible = visible;
    refresh();
}

void Tray::refresh() {
    if (!m_icon)
        return;

    // The tooltip is the only place a background app can say "I am alive"
    // without opening anything or making a sound, and how many machines could
    // actually take a file right now is the question it can answer.
    QString state = tr("Taildrop — listening");
    if (m_receiving && m_peers > 0)
        state = tr("Taildrop — listening, %n device(s)", "", m_peers);
    m_icon->setToolTip(m_receiving ? state : tr("Taildrop — offline"));
    m_status->setText(m_receiving ? state : tr("Offline"));
    m_toggle->setText(m_visible ? tr("Hide window") : tr("Open Taildrop"));
}

void Tray::announceArrival(const QString &fileName) {
    if (!m_icon || m_visible)
        return;
    // Baloo and friends already send a notification per received file on
    // some setups; this only fires when the window is hidden, which no other
    // part of the app can see.
    if (QSystemTrayIcon::supportsMessages())
        m_icon->showMessage(tr("Taildrop"), tr("Received %1").arg(fileName),
                            taildropIcon(), 5000);
}
