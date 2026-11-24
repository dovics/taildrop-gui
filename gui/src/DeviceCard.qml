import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material

// One device on the tailnet. This is the drop target: dropping files here
// sends them to that device, which is the whole point of the app.
Item {
    id: card

    required property string hostname
    required property string os
    required property bool online
    required property bool canReceive
    required property string path
    required property string ipv4

    signal filesDropped(var urls)

    property bool hovered: false

    implicitWidth: 240
    implicitHeight: 132

    Rectangle {
        anchors.fill: parent
        radius: Math.min(theme.radius, 8)
        color: card.hovered && card.canReceive
               ? Qt.rgba(0.478, 0.635, 0.968, 0.14)
               : theme.surface
        border.width: card.hovered && card.canReceive ? 2 : 1
        border.color: card.hovered && card.canReceive
                      ? theme.accent
                      : Qt.rgba(1, 1, 1, 0.08)
        opacity: card.online ? 1.0 : 0.45

        Behavior on color { ColorAnimation { duration: 90 } }

        Column {
            anchors.fill: parent
            anchors.margins: 16
            spacing: 6

            Row {
                spacing: 10
                width: parent.width

                Rectangle {
                    width: 28
                    height: 28
                    radius: Math.min(4, theme.radius)
                    color: Qt.rgba(1, 1, 1, 0.06)

                    Label {
                        anchors.centerIn: parent
                        text: card.osIcon(card.os)
                        color: theme.accent
                        font.pixelSize: 13
                        font.weight: Font.DemiBold
                    }
                }

                Column {
                    spacing: 2
                    width: parent.width - 40

                    Label {
                        text: card.hostname
                        color: theme.foreground
                        font.pixelSize: 15
                        font.weight: Font.Medium
                        elide: Text.ElideRight
                        width: parent.width
                    }

                    Label {
                        text: card.ipv4
                        color: theme.muted
                        font.pixelSize: 11
                        elide: Text.ElideRight
                        width: parent.width
                    }
                }
            }

            Item { width: 1; height: 2 }

            Label {
                text: !card.online
                      ? qsTr("Offline")
                      : card.canReceive
                        ? qsTr("Drop files to send")
                        : qsTr("Not receiving")
                color: card.online && card.canReceive ? theme.accent : theme.muted
                font.pixelSize: 11
            }

            Label {
                text: card.path
                color: theme.muted
                font.pixelSize: 10
                visible: card.path !== ""
            }
        }
    }

    HoverHandler {
        onHoveredChanged: card.hovered = hovered
    }

    // Qt 6 file drops arrive as URLs; the engine takes local paths.
    //
    // On Windows a URL looks like file:///C:/dir/a.txt, and stripping the
    // scheme by hand leaves /C:/dir/a.txt, which is not a valid path there.
    // QUrl.toLocalFile knows the platform rules, so use it instead.
    DropArea {
        anchors.fill: parent
        enabled: card.online && card.canReceive
        onDropped: (drop) => {
            if (!drop.hasUrls)
                return
            const paths = []
            for (let i = 0; i < drop.urls.length; ++i) {
                const local = drop.urls[i].toLocalFile()
                if (local.length > 0)
                    paths.push(local)
            }
            if (paths.length > 0)
                card.filesDropped(paths)
        }
    }

    // Letters rather than emoji: platform emoji fall back to tofu on minimal
    // font installs, and a missing glyph in the device list looks like a bug.
    function osIcon(os) {
        const name = (os || "").toLowerCase()
        if (name === "windows") return "W"
        if (name === "macos") return "M"
        if (name === "ios") return "i"
        if (name === "linux") return "L"
        if (name === "android") return "A"
        return "?"
    }
}
