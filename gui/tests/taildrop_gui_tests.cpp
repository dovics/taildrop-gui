#include <QDir>
#include <QFile>
#include <QTemporaryDir>
#include <QtTest>

#include "backend.h"
#include "singleinstance.h"
#include "theme.h"

// Covers what the GUI layer owns: reading the theme, and turning the engine's
// output into state. The protocol itself is tested in Rust, where it lives.
class TaildropTests : public QObject {
    Q_OBJECT

private:
    // A stand-in for the Rust engine, so the scan schedule and the announce
    // path can be observed without a tailnet. Each `peers --json` call appends
    // a line to a counter file; `serve` says it is ready, and in announcing
    // mode also reports a peer coming online a second later.
    QString m_fakeEngine;
    QTemporaryDir m_temp;

    QString makeFakeEngine(bool announcing)
    {
#ifdef Q_OS_UNIX
        const QString dir = m_temp.path();
        m_fakeEngine = dir + "/taildrop-cli";
        const QString counter = dir + (announcing ? "/announce-count" : "/scan-count");
        QFile::remove(counter);

        QString script = QStringLiteral(
            "#!/bin/sh\n"
            "case \"$1\" in\n"
            "  peers) echo x >> %1; echo '[]'; exit 0 ;;\n"
            "  announce) exit 0 ;;\n"
            "  serve)\n"
            "    echo '{\"event\":\"ready\",\"alias\":\"fake\",\"fingerprint\":\"AB\",\"inbox\":\"/tmp\",\"port\":53318}'\n")
            .arg(counter);
        if (announcing) {
            // Printed once the receiver has been up a moment, exactly as a peer
            // starting on another machine would.
            script += QStringLiteral(
                "    ( sleep 1; echo '{\"event\":\"announce\",\"alias\":\"peer\",\"fromAddr\":\"100.1.1.1:1\",\"fingerprint\":\"CD\"}' ) &\n");
        }
        script += QStringLiteral(
            "    cat > /dev/null\n"
            "    exit 0 ;;\n"
            "esac\n"
            "exit 1\n");

        QFile file(m_fakeEngine);
        if (!file.open(QIODevice::WriteOnly))
            return {};
        file.write(script.toUtf8());
        file.close();
        QFile::setPermissions(
            m_fakeEngine, QFile::ReadOwner | QFile::WriteOwner | QFile::ExeOwner);
        return counter;
#else
        Q_UNUSED(announcing)
        return {};
#endif
    }

    static int scanCount(const QString &counter)
    {
        QFile file(counter);
        if (!file.open(QIODevice::ReadOnly))
            return 0;
        return int(file.readAll().count('\n'));
    }

private slots:
    void themeFallsBack()
    {
        QTemporaryDir dir;
        Theme theme(dir.path());
        QCOMPARE(theme.accent(), QString("#FFD60A"));
        QCOMPARE(theme.accentForeground(), QString("black"));
        // Without a theme file the window must still be readable.
        QVERIFY(!theme.background().isEmpty());
        QVERIFY(!theme.foreground().isEmpty());
    }

    void themeReadsEveryColour()
    {
        QTemporaryDir dir;
        QDir(dir.path()).mkpath("theme");
        QFile colors(dir.path() + "/theme/colors.toml");
        QVERIFY(colors.open(QIODevice::WriteOnly));
        colors.write("accent = \"#112233\"\n"
                     "background = \"#0a0b0c\"\n"
                     "foreground = \"#dddeee\"\n"
                     "dark_foreground = \"#556677\"\n"
                     "lighter_background = \"#1f2020\"\n");
        colors.close();

        Theme theme(dir.path());
        QCOMPARE(theme.accent(), QString("#112233"));
        QCOMPARE(theme.accentForeground(), QString("white"));
        QCOMPARE(theme.background(), QString("#0a0b0c"));
        QCOMPARE(theme.foreground(), QString("#dddeee"));
        QCOMPARE(theme.muted(), QString("#556677"));
        QCOMPARE(theme.surface(), QString("#1f2020"));
    }

    void themeFollowsHyprlandRounding()
    {
        QTemporaryDir dir;
        const QString response = dir.filePath("response.json"),
                      command = dir.filePath("hyprctl");
        auto write = [](const QString &path, const QByteArray &data) {
            QFile file(path);
            return file.open(QIODevice::WriteOnly) && file.write(data) == data.size();
        };
        QVERIFY(write(command, "#!/bin/sh\ncat '" + response.toUtf8() + "'\n"));
        QVERIFY(QFile::setPermissions(
            command, QFile::ReadOwner | QFile::WriteOwner | QFile::ExeOwner));
        QVERIFY(write(response, "{\"int\":0}"));

        Theme theme(dir.path(), nullptr, command);
        QCOMPARE(theme.radius(), 0);
        QVERIFY(write(response, "{\"int\":6}"));
        QTRY_COMPARE(theme.radius(), 6);
    }

    void accentForegroundContrasts()
    {
        QTemporaryDir dir;
        QDir(dir.path()).mkpath("theme");
        // A pale accent needs black text; a deep one needs white.
        for (const auto &[accent, expected] :
             QList<QPair<QString, QString>>{{"#FFD60A", "black"},
                                            {"#101820", "white"}}) {
            QFile colors(dir.path() + "/theme/colors.toml");
            QVERIFY(colors.open(QIODevice::WriteOnly));
            colors.write(QString("accent = \"%1\"\n").arg(accent).toUtf8());
            colors.close();
            Theme theme(dir.path());
            QCOMPARE(theme.accentForeground(), expected);
        }
    }

    void enginePathHonoursOverride()
    {
        qputenv("TAILDROP_ENGINE", "/tmp/does-not-need-to-exist");
        QCOMPARE(Backend::findEngine(), QString("/tmp/does-not-need-to-exist"));
        qunsetenv("TAILDROP_ENGINE");
    }

    void backendStartsEmpty()
    {
        Backend backend;
        QVERIFY(!backend.receiving());
        QVERIFY(backend.peers().isEmpty());
        QVERIFY(backend.transfers().isEmpty());
    }

    void unknownPeerResolvesToNothing()
    {
        Backend backend;
        QVERIFY(backend.peerByName("nobody").isEmpty());
    }

    // Discovery must not depend on the user pressing Rescan. A fake engine
    // counts the scans it is asked for, which is the only externally visible
    // sign that the timer and the announce path both fire.
    void rescanCycleScansWithoutBeingAsked()
    {
        const QString counter = makeFakeEngine(false);
        if (counter.isEmpty())
            QSKIP("needs a POSIX shell to fake the engine");
        qputenv("TAILDROP_ENGINE", m_fakeEngine.toUtf8());

        Backend backend;
        backend.setRescanIntervalMs(150);
        backend.startRescanCycle();

        // Two ticks, not one: proves the timer repeats rather than firing once.
        QTRY_VERIFY_WITH_TIMEOUT(scanCount(counter) >= 2, 5000);
        qunsetenv("TAILDROP_ENGINE");
    }

    void peerAnnounceTriggersARescan()
    {
        const QString counter = makeFakeEngine(true);
        if (counter.isEmpty())
            QSKIP("needs a POSIX shell to fake the engine");
        qputenv("TAILDROP_ENGINE", m_fakeEngine.toUtf8());

        Backend backend;
        // No periodic timer at all: any scan can only have come from the
        // announce the fake receiver emits a second after going ready.
        backend.startAutoReceive();
        QCOMPARE(scanCount(counter), 0);

        QTRY_VERIFY_WITH_TIMEOUT(scanCount(counter) >= 1, 8000);
        backend.stopReceiving();
        qunsetenv("TAILDROP_ENGINE");
    }

    // A second launch must reach the first instead of starting a receiver that
    // cannot bind the port. The socket is the whole mechanism, so test it
    // directly rather than through a window.
    void secondLaunchReachesTheFirst()
    {
        const QString key = "taildrop-test-" + QString::number(QCoreApplication::applicationPid());
        SingleInstance first(key);
        QVERIFY(first.claim());

        QSignalSpy shown(&first, &SingleInstance::showRequested);

        // A later launch: it loses the claim and the running instance is asked
        // to come forward.
        SingleInstance second(key);
        QVERIFY(!second.claim(true));
        QTRY_COMPARE_WITH_TIMEOUT(shown.count(), 1, 3000);

        // An autostart that finds the app already running is told the same
        // thing, but must not pull the window in front of what the user is
        // doing.
        SingleInstance autostart(key);
        QVERIFY(!autostart.claim(false));
        QTest::qWait(300);
        QCOMPARE(shown.count(), 1);
    }

    // The drop handler converts dropped URLs to paths for the Rust engine.
    // Doing that by string surgery is wrong on Windows, where the result would
    // be /C:/dir/a.txt; QUrl::toLocalFile applies the platform's own rules, so
    // pin what it actually guarantees on the platform running the test.
    void droppedUrlsBecomePlatformPaths()
    {
        const QUrl spaced("file:///home/me/My%20File.txt");
        QCOMPARE(spaced.toLocalFile(), QString("/home/me/My File.txt"));

        // A non-file URL must not silently turn into a bogus path.
        QVERIFY(QUrl("https://example.com/x").toLocalFile().isEmpty());

#ifdef Q_OS_WIN
        QCOMPARE(QUrl("file:///C:/Users/me/a.txt").toLocalFile(),
                 QString("C:/Users/me/a.txt"));
#else
        // Off Windows the drive letter is just a path component; the important
        // property is that no scheme text survives, which hand-parsing broke.
        const QString converted = QUrl("file:///C:/Users/me/a.txt").toLocalFile();
        QVERIFY(!converted.startsWith("file:"));
        QVERIFY(converted.endsWith("a.txt"));
#endif
    }
};

QTEST_MAIN(TaildropTests)
#include "taildrop_gui_tests.moc"
