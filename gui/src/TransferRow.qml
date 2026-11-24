import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material

// One line in the transfer list: what moved, where, and whether it worked.
Item {
    id: row

    implicitHeight: 56

    Row {
        anchors.fill: parent
        anchors.leftMargin: 4
        anchors.rightMargin: 4
        spacing: 12

        // Direction: in or out. A glyph would be nicer but this reads at any
        // font size and never renders as tofu.
        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 26
            height: 26
            radius: Math.min(4, theme.radius)
            color: Qt.rgba(1, 1, 1, 0.06)

            Label {
                anchors.centerIn: parent
                text: modelData.direction === "in" ? "↓" : "↑"
                color: modelData.state === "failed" ? "#f7768e" : theme.accent
                font.pixelSize: 15
                font.weight: Font.DemiBold
            }
        }

        Column {
            anchors.verticalCenter: parent.verticalCenter
            width: parent.width - 26 - 12 - 110
            spacing: 3

            Label {
                text: modelData.fileName
                color: theme.foreground
                font.pixelSize: 13
                elide: Text.ElideMiddle
                width: parent.width
            }

            Label {
                text: {
                    const parts = []
                    if (modelData.direction === "in")
                        parts.push(qsTr("From %1").arg(modelData.fromAlias || "?"))
                    else if (modelData.to)
                        parts.push(qsTr("To %1").arg(modelData.to))
                    if (modelData.state === "failed")
                        parts.push(qsTr("Failed"))
                    else if (modelData.state === "done")
                        parts.push(qsTr("Done"))
                    else
                        parts.push(qsTr("Sending"))
                    return parts.join(" · ")
                }
                color: modelData.state === "failed" ? "#f7768e" : theme.muted
                font.pixelSize: 11
                elide: Text.ElideRight
                width: parent.width
            }
        }

        Item { width: 1; height: 1 }

        Column {
            anchors.verticalCenter: parent.verticalCenter
            width: 110
            spacing: 4

            Label {
                anchors.right: parent.right
                text: row.sizeText(modelData.size || modelData.totalBytes || 0)
                color: theme.muted
                font.pixelSize: 11
            }

            Rectangle {
                width: parent.width
                height: 3
                radius: 1.5
                color: Qt.rgba(1, 1, 1, 0.08)
                visible: modelData.state === "sending"

                Rectangle {
                    width: parent.width * (modelData.progress || 0)
                    height: parent.height
                    radius: 1.5
                    color: theme.accent
                }
            }
        }
    }

    function sizeText(bytes) {
        if (!bytes || bytes <= 0) return ""
        const units = ["B", "KiB", "MiB", "GiB", "TiB"]
        let v = bytes, i = 0
        while (v >= 1024 && i < units.length - 1) { v /= 1024; ++i }
        return i === 0 ? v + " B" : v.toFixed(1) + " " + units[i]
    }
}
