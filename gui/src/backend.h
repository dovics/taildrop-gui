#pragma once

#include <QJsonArray>
#include <QObject>
#include <QElapsedTimer>
#include <QProcess>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>

// The app's state and work. The transfer engine itself lives in the Rust
// binary; this class drives it and turns its output into properties QML can
// bind to. Nothing here reimplements the protocol — that would be two sources
// of truth for the same wire format.
class Backend : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool receiving READ receiving NOTIFY receivingChanged)
    Q_PROPERTY(QString fingerprint READ fingerprint NOTIFY fingerprintChanged)
    Q_PROPERTY(QString inbox READ inbox NOTIFY inboxChanged)
    Q_PROPERTY(QVariantList peers READ peers NOTIFY peersChanged)
    Q_PROPERTY(QVariantList transfers READ transfers NOTIFY transfersChanged)
    Q_PROPERTY(bool scanning READ scanning NOTIFY scanningChanged)
    Q_PROPERTY(QString statusText READ statusText NOTIFY statusTextChanged)
    Q_PROPERTY(QString enginePath READ enginePath NOTIFY enginePathChanged)

public:
    explicit Backend(QObject *parent = nullptr);
    ~Backend() override;

    bool receiving() const { return m_receiving; }
    QString fingerprint() const { return m_fingerprint; }
    QString inbox() const { return m_inbox; }
    QVariantList peers() const { return m_peers; }
    QVariantList transfers() const { return m_transfers; }
    bool scanning() const { return m_scanning; }
    QString statusText() const { return m_statusText; }
    QString enginePath() const { return m_engine; }

    // Where the Rust binary is: $TAILDROP_ENGINE, else beside us, else PATH.
    static QString findEngine();

    // Listening is not a mode the user switches on: the app is a receiver for
    // as long as it is open. startAutoReceive() is called once at startup and
    // brings the receiver back if it ever dies.
    Q_INVOKABLE void startAutoReceive();

    Q_INVOKABLE void startReceiving(const QString &inbox = {}, int port = 0,
                                   const QString &pin = {});
    Q_INVOKABLE void stopReceiving();
    Q_INVOKABLE void scanPeers();
    Q_INVOKABLE void sendFiles(const QStringList &paths, const QString &toPeer,
                               int port = 0, const QString &protocol = {});
    Q_INVOKABLE QVariantMap peerByName(const QString &name) const;

signals:
    void receivingChanged();
    void fingerprintChanged();
    void inboxChanged();
    void peersChanged();
    void transfersChanged();
    void scanningChanged();
    void statusTextChanged();
    void enginePathChanged();
    void errorOccurred(const QString &message);

private:
    void setStatus(const QString &text);
    void scheduleReceiverRestart();
    void handleServeLine(const QString &line);
    void handlePeersOutput(const QString &output);
    void watchTransfer(QProcess *process, const QVariantMap &transfer);

    QString m_engine;
    bool m_receiving = false;
    QString m_fingerprint;
    QString m_inbox;
    QVariantList m_peers;
    QVariantList m_transfers;
    bool m_scanning = false;
    QString m_statusText;

    int m_sendSequence = 0;
    bool m_autoReceive = false;
    bool m_shuttingDown = false;
    int m_restartCount = 0;
    QProcess *m_serve = nullptr;
    QProcess *m_scan = nullptr;
    QTimer *m_restart = nullptr;
    QElapsedTimer m_startedAt;
    QByteArray m_serveBuffer;
};
