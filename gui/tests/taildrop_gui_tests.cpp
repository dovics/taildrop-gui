#include <QDir>
#include <QFile>
#include <QTemporaryDir>
#include <QtTest>

#include "backend.h"
#include "theme.h"

// Covers what the GUI layer owns: reading the theme, and turning the engine's
// output into state. The protocol itself is tested in Rust, where it lives.
class TaildropTests : public QObject {
    Q_OBJECT

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
};

QTEST_MAIN(TaildropTests)
#include "taildrop_gui_tests.moc"
