#pragma once

#include <QJsonArray>
#include <QObject>
#include <QElapsedTimer>
#include <QProcess>
#include <QTimer>
#include <QUrl>
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

    // Discovery runs itself: a scan shortly after launch, another as soon as the
    // receiver is up, then a periodic one, plus an immediate one whenever a peer
    // announces it just came online. Manual rescans (button, Ctrl+R) reuse the
    // same path, so there is one definition of "refresh the list".
    Q_INVOKABLE void startRescanCycle();
    Q_INVOKABLE void announceOnline();
    // Used by the tests to keep a periodic-scan check short; the app leaves it
    // at the default.
    Q_INVOKABLE void setRescanIntervalMs(int ms);

    Q_INVOKABLE void startReceiving(const QString &inbox = {}, int port = 0,
                                   const QString &pin = {});
    Q_INVOKABLE void stopReceiving();
    Q_INVOKABLE void scanPeers();
    Q_INVOKABLE void sendFiles(const QStringList &paths, const QString &toPeer,
                               int port = 0, const QString &protocol = {});
    Q_INVOKABLE QVariantMap peerByName(const QString &name) const;

    // QML's url type has no toLocalFile(); it is a QUrl method. Hand the URL to
    // C++ so the platform rules apply instead of string surgery, which on
    // Windows turns file:///C:/x into /C:/x.
    Q_INVOKABLE QString toLocalPath(const QUrl &url) const;

    // Append-only diagnostics, because a GUI started from the launcher has no
    // terminal to print to and "nothing happened" is otherwise unfalsifiable.
    Q_INVOKABLE void logLine(const QString &line);

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
    // A file finished arriving. The window shows it in the list; the tray uses
    // it for a notification when the window is hidden, since a background
    // receiver that stays silent is indistinguishable from a dead one.
    void receivedArrival(const QString &fileName);

private:
    void setStatus(const QString &text);
    void scheduleReceiverRestart();
    void stopServeProcess();
    void handleServeLine(const QString &line);
    void handlePeersOutput(const QString &output);
    void watchTransfer(QProcess *process, const QVariantMap &transfer);
    void pingPeersForRescan();

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
    QString m_logPath;
    QByteArray m_serveBuffer;

    QTimer *m_rescanTimer = nullptr;
    // Announcing is started only after the receiver answers "ready": a peer
    // that rescans the moment we announce must find us actually listening.
    bool m_announced = false;
    // Coalesces a burst of announces (several machines starting at once) into
    // one scan, instead of one scan each.
    QTimer *m_rescanCoalesce = nullptr;
    int m_rescanIntervalMs = 30000;
};
