#include "backend.h"

#include <QCoreApplication>
#include <algorithm>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QStandardPaths>
#include <QTimer>

Backend::Backend(QObject *parent) : QObject(parent) {
    m_engine = findEngine();
    emit enginePathChanged();
}

// One line per event, appended to a file under the state directory.
// The launcher gives the app no terminal, so without this a failed drop leaves
// no trace at all and the only symptom is "nothing happened".
void Backend::logLine(const QString &line) {
    if (m_logPath.isEmpty()) {
        const QString dir = QStandardPaths::writableLocation(
            QStandardPaths::GenericStateLocation);
        QDir().mkpath(dir);
        m_logPath = dir + "/taildrop.log";
    }
    QFile file(m_logPath);
    if (!file.open(QIODevice::WriteOnly | QIODevice::Append | QIODevice::Text))
        return;
    const QString stamped = QDateTime::currentDateTime().toString(Qt::ISODate)
                            + " " + line + "\n";
    file.write(stamped.toUtf8());
}

Backend::~Backend() {
    m_shuttingDown = true;
    if (m_restart) {
        m_restart->stop();
        m_restart->deleteLater();
        m_restart = nullptr;
    }
    stopServeProcess();
}

// The receiver is the point of the app, so it starts with the window instead of
// waiting for a button. Every path that ends the receiver funnels back here.
void Backend::startAutoReceive() {
    m_autoReceive = true;
    if (m_receiving || m_serve)
        return;
    startReceiving({}, 0, {});
}

void Backend::scheduleReceiverRestart() {
    if (!m_autoReceive || m_shuttingDown)
        return;
    if (m_restart)
        return;

    // Back off on repeated failures so a broken setup does not spin: 1s, 2s,
    // 4s, capped at 30s. A run that lasted a while resets the counter.
    const int delay = std::min(30000, 1000 << std::min(m_restartCount, 5));
    m_restartCount = std::min(m_restartCount + 1, 6);

    m_restart = new QTimer(this);
    m_restart->setSingleShot(true);
    m_restart->setInterval(delay);
    connect(m_restart, &QTimer::timeout, this, [this] {
        m_restart->deleteLater();
        m_restart = nullptr;
        if (m_autoReceive && !m_shuttingDown)
            startReceiving({}, 0, {});
    });
    m_restart->start();
}

QString Backend::findEngine() {
    const QByteArray override = qgetenv("TAILDROP_ENGINE");
    if (!override.isEmpty())
        return QString::fromUtf8(override);

    // Beside the GUI binary first, so a built tree works without installing.
    // Windows appends .exe, and a missing suffix there means the engine is
    // never found next to the app — the exact thing this lookup is for.
    const QString suffix =
#ifdef Q_OS_WIN
        QStringLiteral(".exe");
#else
        QString();
#endif
    const QString beside =
        QCoreApplication::applicationDirPath() + "/taildrop-cli" + suffix;
    if (QFileInfo::exists(beside))
        return beside;

    for (const auto &candidate : {QStringLiteral("taildrop-cli")}) {
        const QString found = QStandardPaths::findExecutable(candidate);
        if (!found.isEmpty())
            return found;
    }
    return {};
}

void Backend::setStatus(const QString &text) {
    if (text == m_statusText)
        return;
    m_statusText = text;
    emit statusTextChanged();
}

void Backend::startReceiving(const QString &inbox, int port, const QString &pin) {
    if (m_receiving)
        return;
    if (m_engine.isEmpty()) {
        emit errorOccurred(tr("The transfer engine is missing. Build it with "
                              "`cargo build --release` in the taildrop-gui folder."));
        return;
    }

    QStringList args{"serve", "--json"};
    if (!inbox.isEmpty())
        args << "--inbox" << inbox;
    if (port > 0)
        args << "--port" << QString::number(port);
    if (!pin.isEmpty())
        args << "--pin" << pin;

    m_serve = new QProcess(this);
    m_serve->setProgram(m_engine);
    m_serve->setArguments(args);
    m_serveBuffer.clear();

    connect(m_serve, &QProcess::readyReadStandardOutput, this, [this] {
        m_serveBuffer += m_serve->readAllStandardOutput();
        // Events are newline-delimited JSON; a partial line stays buffered.
        int nl;
        while ((nl = m_serveBuffer.indexOf('\n')) >= 0) {
            const QByteArray line = m_serveBuffer.left(nl);
            m_serveBuffer.remove(0, nl + 1);
            handleServeLine(QString::fromUtf8(line).trimmed());
        }
    });

    connect(m_serve, &QProcess::readyReadStandardError, this, [this] {
        const QString err = QString::fromUtf8(m_serve->readAllStandardError()).trimmed();
        if (!err.isEmpty())
            setStatus(err);
    });

    connect(m_serve, &QProcess::finished, this,
            [this](int code, QProcess::ExitStatus) {
                m_receiving = false;
                emit receivingChanged();
                m_serve->deleteLater();
                m_serve = nullptr;

                // Exit 15 is our own terminate(); anything else is a crash or a
                // port that was taken. Both are retried by scheduleReceiverRestart.
                if (m_shuttingDown || code == 15)
                    return;
                if (code != 0)
                    setStatus(tr("Receiver stopped (exit %1), restarting…").arg(code));
                scheduleReceiverRestart();
            });

    // Give the engine a stdin we own and then close on quit: that is the
    // portable "stop" signal (Windows terminate() is abrupt).
    m_serve->setProcessChannelMode(QProcess::SeparateChannels);
    m_serve->setReadChannel(QProcess::StandardOutput);

    m_serve->start();
    if (!m_serve->waitForStarted(4000)) {
        emit errorOccurred(tr("Could not start the receiver."));
        m_serve->deleteLater();
        m_serve = nullptr;
        return;
    }
    m_receiving = true;
    emit receivingChanged();
    setStatus(tr("Starting receiver…"));
    m_startedAt.start();
}

void Backend::handleServeLine(const QString &line) {
    if (line.isEmpty())
        return;
    const auto doc = QJsonDocument::fromJson(line.toUtf8());
    if (!doc.isObject())
        return;
    const auto obj = doc.object();
    const QString event = obj.value("event").toString();

    if (event == "ready") {
        // A receiver that has been up for a while starts its backoff from zero.
        if (m_startedAt.isValid() && m_startedAt.elapsed() > 60000L)
            m_restartCount = 0;
        m_fingerprint = obj.value("fingerprint").toString();
        m_inbox = obj.value("inbox").toString();
        emit fingerprintChanged();
        emit inboxChanged();
        setStatus(tr("Receiving on port %1").arg(obj.value("port").toInt()));
        return;
    }

    if (event == "received") {
        // Newest first: a receiving window should show the last arrival.
        QVariantMap received;
        received["fileName"] = obj.value("fileName").toString();
        received["path"] = obj.value("path").toString();
        received["size"] = obj.value("size").toVariant();
        received["fromAlias"] = obj.value("fromAlias").toString();
        received["verified"] = obj.value("verified").toBool();
        received["elapsedMs"] = obj.value("elapsedMs").toVariant();
        received["direction"] = "in";
        received["state"] = "done";
        received["progress"] = 1.0;
        received["at"] = QDateTime::currentDateTime().toString(Qt::ISODate);
        m_transfers.prepend(received);
        emit transfersChanged();
        setStatus(tr("Received %1").arg(received.value("fileName").toString()));
    }
}

void Backend::stopReceiving() {
    m_autoReceive = false;
    if (m_restart) {
        m_restart->stop();
        m_restart->deleteLater();
        m_restart = nullptr;
    }
    stopServeProcess();
}

// Ask the engine to finish, then insist.
//
// Closing stdin is the polite signal it listens for in --json mode, and it lets
// an in-flight file finish. terminate() is abrupt on Windows and can leave a
// .part file behind, so it only happens after the polite window expires.
void Backend::stopServeProcess() {
    if (!m_serve)
        return;
    m_serve->closeWriteChannel();
    if (m_serve->waitForFinished(2000))
        return;
    m_serve->terminate();
    if (m_serve->waitForFinished(1500))
        return;
    m_serve->kill();
    m_serve->waitForFinished(1000);
}

void Backend::scanPeers() {
    if (m_scanning || m_engine.isEmpty()) {
        if (m_engine.isEmpty())
            emit errorOccurred(tr("The transfer engine is missing."));
        return;
    }
    m_scanning = true;
    emit scanningChanged();
    setStatus(tr("Looking for devices…"));

    m_scan = new QProcess(this);
    m_scan->setProgram(m_engine);
    m_scan->setArguments({"peers", "--json"});

    connect(m_scan, &QProcess::finished, this,
            [this](int code, QProcess::ExitStatus) {
                const QString out = QString::fromUtf8(m_scan->readAllStandardOutput());
                const QString err = QString::fromUtf8(m_scan->readAllStandardError()).trimmed();
                m_scan->deleteLater();
                m_scan = nullptr;
                m_scanning = false;
                emit scanningChanged();
                if (code == 0)
                    handlePeersOutput(out);
                else
                    setStatus(err.isEmpty() ? tr("Could not read the device list.") : err);
            });

    connect(m_scan, &QProcess::errorOccurred, this, [this](QProcess::ProcessError) {
        m_scanning = false;
        emit scanningChanged();
        emit errorOccurred(tr("Could not start the device scan."));
    });

    m_scan->start();
}

void Backend::handlePeersOutput(const QString &output) {
    const auto doc = QJsonDocument::fromJson(output.toUtf8());
    if (!doc.isArray()) {
        setStatus(tr("Unexpected answer from the engine."));
        return;
    }

    QVariantList list;
    for (const auto &entry : doc.array()) {
        const auto peer = entry.toObject();
        QVariantMap item;
        item["hostname"] = peer.value("hostname").toString();
        item["dnsName"] = peer.value("dnsName").toString();
        item["os"] = peer.value("os").toString();
        item["online"] = peer.value("online").toBool();
        item["ipv4"] = peer.value("ipv4").toString();
        item["path"] = peer.value("path").toString();
        const auto server = peer.value("transferServer");
        item["canReceive"] = server.isObject();
        if (server.isObject()) {
            item["port"] = server.toObject().value("port").toInt();
            item["protocol"] = server.toObject().value("protocol").toString();
            item["remoteAlias"] = server.toObject().value("alias").toString();
        }
        list.append(item);
    }

    m_peers = list;
    emit peersChanged();

    int ready = 0;
    for (const auto &p : m_peers)
        if (p.toMap().value("canReceive").toBool())
            ++ready;
    setStatus(ready > 0 ? tr("%1 device(s) can receive").arg(ready)
                        : tr("No device is receiving right now"));
}

QVariantMap Backend::peerByName(const QString &name) const {
    for (const auto &p : m_peers) {
        const auto map = p.toMap();
        if (map.value("hostname").toString().compare(name, Qt::CaseInsensitive) == 0)
            return map;
    }
    return {};
}

// QML's url cannot do this itself: toLocalFile() is a QUrl method and calling
// it from QML throws, which silently aborted the drop handler.
QString Backend::toLocalPath(const QUrl &url) const {
    return url.toLocalFile();
}

void Backend::sendFiles(const QStringList &paths, const QString &toPeer, int port,
                        const QString &protocol) {
    logLine(QStringLiteral("sendFiles: %1 file(s) to %2 port=%3 engine=%4")
                .arg(paths.size())
                .arg(toPeer)
                .arg(port)
                .arg(m_engine.isEmpty() ? QStringLiteral("<missing>") : m_engine));
    if (paths.isEmpty() || toPeer.isEmpty())
        return;
    if (m_engine.isEmpty()) {
        emit errorOccurred(tr("The transfer engine is missing."));
        return;
    }

    const QString name = QFileInfo(paths.first()).fileName();
    const QString id = QStringLiteral("%1-%2")
                           .arg(QDateTime::currentMSecsSinceEpoch())
                           .arg(++m_sendSequence);

    QVariantMap transfer;
    transfer["id"] = id;
    transfer["fileName"] = paths.size() == 1
                               ? name
                               : tr("%1 files").arg(paths.size());
    transfer["to"] = toPeer;
    transfer["state"] = "sending";
    transfer["direction"] = "out";
    transfer["progress"] = 0.0;
    transfer["count"] = paths.size();
    transfer["totalBytes"] = 0;
    transfer["at"] = QDateTime::currentDateTime().toString(Qt::ISODate);
    m_transfers.prepend(transfer);
    emit transfersChanged();

    QStringList args{"send"};
    args << paths;
    args << "--to" << toPeer;
    // Use the port the scan found, not the engine's default: a peer may run
    // Taildrop, LocalSend, or both, and only the scan knows which answered.
    if (port <= 0)
        port = peerByName(toPeer).value("port").toInt();
    if (port > 0)
        args << "--port" << QString::number(port);
    if (!protocol.isEmpty())
        args << "--tls" << protocol;

    auto *process = new QProcess(this);
    process->setProgram(m_engine);
    process->setArguments(args);

    // The engine prints a progress bar on stdout; watch stderr for the verdict
    // instead of parsing it, and use the exit code as the result.
    connect(process, &QProcess::finished, this,
            [this, process, id, toPeer](int code, QProcess::ExitStatus) {
                const QString err =
                    QString::fromUtf8(process->readAllStandardError()).trimmed();
                process->deleteLater();

                for (int i = 0; i < m_transfers.size(); ++i) {
                    auto map = m_transfers[i].toMap();
                    if (map.value("id").toString() != id)
                        continue;
                    map["state"] = code == 0 ? "done" : "failed";
                    map["progress"] = code == 0 ? 1.0 : 0.0;
                    if (!err.isEmpty())
                        map["error"] = err;
                    m_transfers[i] = map;
                    break;
                }
                emit transfersChanged();

                const QString reason = err.isEmpty() ? tr("Send failed") : err;
                setStatus(code == 0 ? tr("Sent to %1").arg(toPeer) : reason);
                if (code != 0)
                    emit errorOccurred(reason);
            });

    connect(process, &QProcess::errorOccurred, this,
            [this, id, toPeer](QProcess::ProcessError) {
                // Could not even start: without this the row would sit at
                // "sending" forever, which reads as a hang, not a failure.
                for (int i = 0; i < m_transfers.size(); ++i) {
                    auto map = m_transfers[i].toMap();
                    if (map.value("id").toString() != id)
                        continue;
                    map["state"] = "failed";
                    m_transfers[i] = map;
                    break;
                }
                emit transfersChanged();
                const QString reason = tr("Could not start the transfer engine.");
                setStatus(reason);
                emit errorOccurred(reason);
                Q_UNUSED(toPeer)
            });

    process->start();
}
