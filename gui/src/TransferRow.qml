import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

// One line in the transfer list: what moved, where, and whether it worked.
Item {
    id: row

    implicitHeight: 56

    // A plain Row cannot subtract for itself: the fixed widths and spacings used
    // to add up to more than the row, which pushed the size column past the
    // right edge and clipped the unit ("12.3 MiB" showing as "12.3 Mi").
    // RowLayout does the arithmetic, so the name column gives up the space.
    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 4
        anchors.rightMargin: 4
        spacing: 12

        // Direction: in or out. A glyph would be nicer but this reads at any
        // font size and never renders as tofu.
        Rectangle {
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredWidth: 26
            Layout.preferredHeight: 26
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

        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignVCenter
            spacing: 3

            Label {
                text: modelData.fileName
                color: theme.foreground
                font.pixelSize: 13
                elide: Text.ElideMiddle
                Layout.fillWidth: true
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
                Layout.fillWidth: true
            }
        }

        // Width comes from the text itself rather than a guessed number of
        // pixels, so "1023.9 MiB" is never cut off, and the column still grows
        // with the font when the desktop is scaled up.
        ColumnLayout {
            Layout.alignment: Qt.AlignVCenter
            spacing: 4

            Label {
                text: row.sizeText(modelData.size || modelData.totalBytes || 0)
                color: theme.muted
                font.pixelSize: 11
                Layout.alignment: Qt.AlignRight
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.minimumWidth: 64
                Layout.preferredHeight: 3
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
