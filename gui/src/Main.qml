import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

ApplicationWindow {
    id: win
    width: 1040
    height: 720
    minimumWidth: 720
    minimumHeight: 520
    visible: true
    title: "Taildrop"
    color: theme.background

    Material.theme: Material.Dark
    Material.accent: theme.accent
    Material.roundedScale: theme.radius

    property bool helpVisible: false

    // Closing the window must take the receiver with it: on Windows the child
    // process has no controlling terminal, so an orphaned engine would keep
    // port 53318 bound and the next launch would look broken.
    onClosing: backend.stopReceiving()

    Shortcut { sequence: "?"; context: Qt.ApplicationShortcut; onActivated: win.helpVisible = !win.helpVisible }
    Shortcut { sequence: "Q"; context: Qt.ApplicationShortcut; onActivated: Qt.quit() }
    Shortcut { sequence: "R"; context: Qt.ApplicationShortcut; onActivated: backend.scanPeers() }
    Shortcut { sequence: "Space"; context: Qt.ApplicationShortcut; onActivated: backend.scanPeers() }

    Item {
        anchors.fill: parent
        LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
        LayoutMirroring.childrenInherit: true

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 24
            spacing: 18

            // ---- header -------------------------------------------------
            RowLayout {
                Layout.fillWidth: true
                spacing: 12

                ColumnLayout {
                    spacing: 2

                    Label {
                        text: "Taildrop"
                        color: theme.foreground
                        font.pixelSize: 22
                        font.weight: Font.DemiBold
                    }

                    Label {
                        text: backend.statusText
                        color: theme.muted
                        font.pixelSize: 12
                        elide: Text.ElideRight
                        Layout.maximumWidth: 520
                    }
                }

                Item { Layout.fillWidth: true }

                Button {
                    text: backend.scanning ? qsTr("Scanning…") : qsTr("Rescan")
                    enabled: !backend.scanning
                    onClicked: backend.scanPeers()
                }

                // The receiver runs on its own; this shows whether it is up
                // rather than asking the user to switch it on.
                Rectangle {
                    id: listenDot
                    width: 8
                    height: 8
                    radius: 4
                    color: backend.receiving ? theme.accent : theme.muted
                    opacity: backend.receiving ? 1.0 : 0.45
                    Layout.alignment: Qt.AlignVCenter

                    SequentialAnimation on opacity {
                        running: backend.receiving
                        loops: Animation.Infinite
                        NumberAnimation { from: 1.0; to: 0.35; duration: 1100 }
                        NumberAnimation { from: 0.35; to: 1.0; duration: 1100 }
                        onStopped: listenDot.opacity = backend.receiving ? 1.0 : 0.45
                    }
                }

                Label {
                    text: backend.receiving ? qsTr("Listening") : qsTr("Offline")
                    color: backend.receiving ? theme.foreground : theme.muted
                    font.pixelSize: 12
                    Layout.alignment: Qt.AlignVCenter
                }
            }

            // ---- devices ------------------------------------------------
            Label {
                text: qsTr("DEVICES")
                color: theme.muted
                font.pixelSize: 10
                font.letterSpacing: 1.2
                visible: deviceGrid.count > 0
            }

            GridView {
                id: deviceGrid
                Layout.fillWidth: true
                Layout.preferredHeight: contentHeight
                Layout.maximumHeight: 300
                cellWidth: 252
                cellHeight: 144
                clip: true
                model: backend.peers

                delegate: Item {
                    width: deviceGrid.cellWidth
                    height: deviceGrid.cellHeight

                    DeviceCard {
                        anchors.fill: parent
                        anchors.margins: 6
                        hostname: modelData.hostname
                        os: modelData.os
                        online: modelData.online
                        canReceive: modelData.canReceive
                        path: modelData.path
                        ipv4: modelData.ipv4
                        onFilesDropped: (urls) => backend.sendFiles(urls, modelData.hostname,
                                                        modelData.port || 0,
                                                        modelData.protocol || "")
                    }
                }

                // Empty state: the window should say what to do, not sit blank.
                Label {
                    anchors.centerIn: parent
                    visible: deviceGrid.count === 0 && !backend.scanning
                    text: qsTr("No devices found. Start Taildrop on another machine on your tailnet.")
                    color: theme.muted
                    font.pixelSize: 13
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    width: win.width * 0.6
                }
            }

            // ---- transfers ---------------------------------------------
            RowLayout {
                Layout.fillWidth: true

                Label {
                    text: qsTr("TRANSFERS")
                    color: theme.muted
                    font.pixelSize: 10
                    font.letterSpacing: 1.2
                }

                Item { Layout.fillWidth: true }

                Label {
                    text: backend.inbox !== "" ? qsTr("Saving to %1").arg(backend.inbox) : ""
                    color: theme.muted
                    font.pixelSize: 10
                    elide: Text.ElideLeft
                    Layout.maximumWidth: 380
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: Math.min(theme.radius, 8)
                color: theme.surface
                border.width: 1
                border.color: Qt.rgba(1, 1, 1, 0.06)
                clip: true

                ListView {
                    id: transferList
                    anchors.fill: parent
                    anchors.margins: 8
                    model: backend.transfers
                    spacing: 2

                    delegate: TransferRow {
                        width: transferList.width
                    }

                    Label {
                        anchors.centerIn: parent
                        visible: transferList.count === 0
                        text: qsTr("Nothing yet. Drop files on a device above to send.")
                        color: theme.muted
                        font.pixelSize: 13
                    }
                }
            }
        }

        // ---- shortcut overlay -------------------------------------------
        Rectangle {
            anchors.fill: parent
            visible: win.helpVisible
            color: Qt.rgba(0, 0, 0, 0.72)

            MouseArea {
                anchors.fill: parent
                onClicked: win.helpVisible = false
            }

            Rectangle {
                anchors.centerIn: parent
                width: 380
                height: helpColumn.height + 48
                radius: Math.min(theme.radius, 10)
                color: theme.surface
                border.width: 1
                border.color: Qt.rgba(1, 1, 1, 0.10)

                Column {
                    id: helpColumn
                    anchors.centerIn: parent
                    spacing: 12

                    Label {
                        text: qsTr("Shortcuts")
                        color: theme.foreground
                        font.pixelSize: 16
                        font.weight: Font.DemiBold
                    }

                    Repeater {
                        model: [
                            { key: "Space / R", what: qsTr("Rescan devices") },
                            { key: "Drag & drop", what: qsTr("Send files to a device") },
                            { key: "?", what: qsTr("This list") },
                            { key: "Q", what: qsTr("Quit") }
                        ]

                        Row {
                            spacing: 16
                            Label {
                                text: modelData.key
                                color: theme.accent
                                font.pixelSize: 12
                                font.family: "monospace"
                                width: 100
                            }
                            Label {
                                text: modelData.what
                                color: theme.foreground
                                font.pixelSize: 12
                            }
                        }
                    }
                }
            }
        }

        // Both engines failing silently would leave a dead window, so surface
        // the reason where the user is looking.
        Connections {
            target: backend
            function onErrorOccurred(message) { toast.show(message) }
        }

        Rectangle {
            id: toast
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 28
            width: Math.min(toastLabel.implicitWidth + 40, win.width - 80)
            height: 44
            radius: Math.min(theme.radius, 8)
            color: "#f7768e"
            opacity: 0
            visible: opacity > 0

            property string message: ""

            function show(text) {
                message = text
                opacity = 1
                hideTimer.restart()
            }

            Label {
                id: toastLabel
                anchors.centerIn: parent
                width: parent.width - 32
                text: toast.message
                color: "#101014"
                font.pixelSize: 12
                elide: Text.ElideRight
            }

            Timer {
                id: hideTimer
                interval: 5000
                onTriggered: toast.opacity = 0
            }

            Behavior on opacity { NumberAnimation { duration: 150 } }
        }
    }

    Component.onCompleted: {
        backend.startAutoReceive()
        backend.scanPeers()
    }
}
