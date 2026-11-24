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
    // Ask before listening: a live instance answers. The way the connect fails
    // is what tells a running app apart from the socket file a crash left
    // behind, and that distinction matters: deleting the file of a live
    // instance would leave it running and unreachable, listening to nothing.
    QLocalSocket probe;
    probe.connectToServer(m_key);
    if (probe.waitForConnected(500)) {
        if (bringForward)
            tellRunningInstanceToShow();
        probe.disconnectFromServer();
        return false;
    }
    if (probe.error() != QLocalSocket::ServerNotFoundError &&
        probe.error() != QLocalSocket::ConnectionRefusedError) {
        // Still busy or starting up, not provably dead: assume it is alive and
        // leave its socket alone rather than risk stranding it.
        if (bringForward)
            tellRunningInstanceToShow();
        return false;
    }

    m_server = new QLocalServer(this);
    QLocalServer::removeServer(m_key);
    if (!m_server->listen(m_key)) {
        // Losing the race with another launch is not an error: that instance
        // owns the name now, so behave as if it had answered.
        if (bringForward)
            tellRunningInstanceToShow();
        return false;
    }

    connect(m_server, &QLocalServer::newConnection, this,
            &SingleInstance::acceptConnection);
    return true;
}

// One connection, one word: "show". A launch without it is an autostart saying
// the app is already fine as it is.
bool SingleInstance::tellRunningInstanceToShow() {
    QLocalSocket socket;
    socket.connectToServer(m_key);
    if (!socket.waitForConnected(500))
        return false;
    socket.write("show");
    socket.flush();
    socket.waitForBytesWritten(500);
    socket.disconnectFromServer();
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
