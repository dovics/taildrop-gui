#include "singleinstance.h"

#include <QLocalServer>
#include <QLocalSocket>

SingleInstance::SingleInstance(const QString &key, QObject *parent)
    : QObject(parent), m_key(key) {}

SingleInstance::~SingleInstance() {
    if (!m_server)
        return;
    m_server->close();
    // A socket file left behind makes the next launch look like a dead process
    // still owns the name.
    QLocalServer::removeServer(m_key);
}

bool SingleInstance::claim(bool bringForward) {
    // Ask before listening: a live instance answers, and that answer is also how
    // a stale socket file left by a crash is told apart from a running app.
    QLocalSocket probe;
    probe.connectToServer(m_key);
    if (probe.waitForConnected(500)) {
        if (bringForward) {
            probe.write("show");
            probe.flush();
            probe.waitForBytesWritten(500);
        }
        probe.disconnectFromServer();
        return false;
    }

    m_server = new QLocalServer(this);
    QLocalServer::removeServer(m_key);
    if (!m_server->listen(m_key)) {
        // Losing the race with another launch is not an error: that instance
        // owns the name now, so behave as if it had answered.
        if (bringForward) {
            QLocalSocket late;
            late.connectToServer(m_key);
            if (late.waitForConnected(500)) {
                late.write("show");
                late.flush();
                late.waitForBytesWritten(500);
                late.disconnectFromServer();
            }
        }
        return false;
    }

    connect(m_server, &QLocalServer::newConnection, this,
            &SingleInstance::acceptConnection);
    return true;
}

void SingleInstance::acceptConnection() {
    while (m_server && m_server->hasPendingConnections()) {
        QLocalSocket *socket = m_server->nextPendingConnection();
        if (!socket)
            break;
        connect(socket, &QLocalSocket::disconnected, socket,
                &QLocalSocket::deleteLater);
        // The message may not have arrived at newConnection() time, so read on
        // readyRead as well as right away.
        connect(socket, &QLocalSocket::readyRead, this,
                [this, socket] { handleSocket(socket); });
        if (socket->bytesAvailable() > 0)
            handleSocket(socket);
    }
}

// Every connection means the same thing: show the window. A launch without that
// word is an autostart saying the app is already fine as it is.
void SingleInstance::handleSocket(QLocalSocket *socket) {
    const QByteArray message = socket->readAll();
    if (message.contains("show"))
        emit showRequested();
    socket->disconnectFromServer();
}
