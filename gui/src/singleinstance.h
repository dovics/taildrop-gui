#pragma once

#include <QObject>
#include <QString>

class QLocalServer;
class QLocalSocket;

// One Taildrop at a time.
//
// A second window is not a second receiver: port 53318 is already bound, so the
// new one would report itself offline and never accept anything, while the tray
// shows two icons that look identical and only one of them works. The first
// instance owns a socket; a later launch reaches it through that socket and
// exits.
class SingleInstance : public QObject {
    Q_OBJECT

public:
    explicit SingleInstance(const QString &key, QObject *parent = nullptr);
    ~SingleInstance() override;

    // True when this process owns the name and should keep running; false when
    // another instance already does, so the caller quits without starting
    // anything.
    //
    // `bringForward` decides what that other instance is told:
    //   - true: the user clicked the dock icon or the launcher, so show the
    //     window ("show me").
    //   - false: the session autostarted us while the app was already running,
    //     so leave whatever is on screen alone ("stay running").
    bool claim(bool bringForward = true);

signals:
    // A later launch asked us to come forward.
    void showRequested();

private:
    void acceptConnection();
    void handleSocket(QLocalSocket *socket);

    QString m_key;
    QLocalServer *m_server = nullptr;
};
