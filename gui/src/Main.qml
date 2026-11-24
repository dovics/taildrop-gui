import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import Qt.labs.platform as Platform

ApplicationWindow {
    id: win
    width: 1040
    height: 720
    minimumWidth: 720
    minimumHeight: 520
    // Started by the session in the background, there is nothing to show yet;
    // bound rather than set after loading so the window never flashes open at
    // login. Hiding it later assigns the property, which is what we want then.
    visible: !startHiddenAtLaunch
    title: "Taildrop"
    color: theme.background

    Material.theme: Material.Dark
    Material.accent: theme.accent
    Material.roundedScale: theme.radius

    property bool helpVisible: false
    // True while files are held over the window, so the whole window can show
    // it is a target instead of leaving the user to find the one small card.
    property bool dragActive: false
    // A deliberate quit must not be turned into "hide", so the tray menu and
    // Ctrl+Q set this before asking the app to exit.
    property bool quitting: false

    // Closing keeps the receiver alive in the background. A file sender has no
    // way to know a window was closed, so quitting on close would silently stop
    // accepting files; Ctrl+Q and the tray's Quit are the deliberate exit.
    // Without a tray there would be no way back, so closing quits there. The
    // receiver itself is tied to the process, not the window, so it keeps
    // running while hidden and stops when the app really exits.
    onClosing: (close) => {
        if (!win.quitting && tray.available && tray.visible) {
            close.accepted = false
            win.hide()
        }
    }

    function requestQuit() {
        win.quitting = true
        Qt.quit()
    }

    function bringForward() {
        win.show()
        win.raise()
        win.requestActivate()
    }

    // A later launch (the dock icon, the launcher) asks this process to come
    // forward instead of starting a second window that could not listen.
    Connections {
        target: instance
        function onShowRequested() { win.bringForward() }
    }

    Shortcut { sequence: "?"; context: Qt.ApplicationShortcut; onActivated: win.helpVisible = !win.helpVisible }
    // ---- tray ---------------------------------------------------------
    // The receiver has to outlive the window: "send me that file" should not
    // depend on the other person happening to have a window open. The tray
    // icon is how a closed window is found again, and it follows the same
    // Material/Omarchy look as the window it opens.
    Platform.SystemTrayIcon {
        id: tray
        // No StatusNotifier host means no icon and no way back to a hidden
        // window, so it stays false there and closing the window quits.
        visible: available
        icon.name: "taildrop-gui"
        tooltip: backend.receiving
                 ? qsTr("Taildrop — listening")
                 : qsTr("Taildrop — offline")

        onActivated: (reason) => {
            if (reason === Platform.SystemTrayIcon.Trigger
                    || reason === Platform.SystemTrayIcon.DoubleClick)
                win.visible ? win.hide() : win.bringForward()
        }

        menu: Platform.Menu {
            Platform.MenuItem {
                text: backend.receiving ? qsTr("Listening") : qsTr("Offline")
                enabled: false
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                text: win.visible ? qsTr("Hide window") : qsTr("Open Taildrop")
                onTriggered: win.visible ? win.hide() : win.bringForward()
            }
            Platform.MenuItem {
                text: qsTr("Rescan devices")
                onTriggered: backend.scanPeers()
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                text: qsTr("Quit")
                onTriggered: win.requestQuit()
            }
        }

        // A background app that never speaks is indistinguishable from one that
        // died, so an arrival announces itself when nobody is looking.
        function announceArrival(name) {
            if (!win.visible && tray.supportsMessages)
                tray.showMessage(qsTr("Taildrop"), qsTr("Received %1").arg(name))
        }
    }

    // Modified keys only: bare R or Space would swallow typing and stop the
    // Space key from activating a focused button.
    Shortcut { sequence: "Ctrl+Q"; context: Qt.ApplicationShortcut; onActivated: win.requestQuit() }
    // Ctrl+W is the "close the window" key people try, and on a background app
    // closing must not stop the receiver.
    Shortcut { sequence: "Ctrl+W"; context: Qt.ApplicationShortcut;
        onActivated: tray.available && tray.visible ? win.hide() : Qt.quit() }
    Shortcut { sequence: "Ctrl+R"; context: Qt.ApplicationShortcut; onActivated: backend.scanPeers() }
    Shortcut { sequence: "F5"; context: Qt.ApplicationShortcut; onActivated: backend.scanPeers() }

    Item {
        anchors.fill: parent
        LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
        LayoutMirroring.childrenInherit: true

        // ---- whole-window drop target -------------------------------
        // A drop target the size of one card means every other pixel of the
        // window rejects the drag, which the cursor shows as "no". The whole
        // window accepts: one device that can receive gets the files directly,
        // several means the cards above are the choice.
        DropArea {
            anchors.fill: parent
            keys: ["text/uri-list"]
            onEntered: win.dragActive = true
            onExited: win.dragActive = false
            onDropped: (drop) => {
                win.dragActive = false

                // Every early exit below used to fail silently, which is
                // indistinguishable from the drop never arriving.
                var urls = []
                if (drop.hasUrls)
                    urls = drop.urls
                backend.logLine("drop received: hasUrls=" + drop.hasUrls +
                                " urls=" + urls.length)

                if (urls.length === 0) {
                    backend.logLine("drop rejected: no URLs on the drop")
                    toast.show(qsTr("That drop carried no files."))
                    return
                }

                const paths = []
                for (let i = 0; i < urls.length; ++i) {
                    // QML's url type has no toLocalFile(); ask C++, which
                    // applies the platform rules (on Windows file:///C:/x is
                    // C:/x, not /C:/x, and hand-parsing gets it wrong).
                    const local = backend.toLocalPath(urls[i])
                    backend.logLine("drop url " + i + ": " + urls[i].toString() +
                                    " -> " + local)
                    if (local.length > 0)
                        paths.push(local)
                }
                if (paths.length === 0) {
                    backend.logLine("drop rejected: no URL resolved to a local path")
                    toast.show(qsTr("That drop carried no local files."))
                    return
                }

                const ready = []
                for (let i = 0; i < backend.peers.length; ++i) {
                    var peer = backend.peers[i]
                    if (peer.online && peer.canReceive)
                        ready.push(peer)
                }
                backend.logLine("drop ready devices: " + ready.length)
                if (ready.length === 1) {
                    var only = ready[0]
                    backend.logLine("drop sending to " + only.hostname +
                                    " port=" + (only.port || 0))
                    backend.sendFiles(paths, only.hostname, only.port || 0,
                                      only.protocol || "")
                } else if (ready.length === 0) {
                    toast.show(qsTr("No device is receiving right now."))
                } else {
                    // Dropping onto a card is the unambiguous choice, and that
                    // card has its own drop target; do not guess here.
                    toast.show(qsTr("Drop on the device you want to send to."))
                }
            }
        }

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
                        onFilesDropped: (urls) => {
                            backend.logLine("card emit " + urls.length + " file(s) for " +
                                            modelData.hostname)
                            backend.sendFiles(urls, modelData.hostname,
                                              modelData.port || 0,
                                              modelData.protocol || "")
                        }
                        onDropRejected: (reason) => toast.show(reason)
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
                color: win.dragActive ? Qt.rgba(0.478, 0.635, 0.968, 0.10)
                                      : theme.surface
                border.width: win.dragActive ? 2 : 1
                border.color: win.dragActive ? theme.accent
                                             : Qt.rgba(1, 1, 1, 0.06)
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
                        text: win.dragActive
                              ? qsTr("Drop to send")
                              : qsTr("Nothing yet. Drop files anywhere in this window.")
                        color: win.dragActive ? theme.accent : theme.muted
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
                            { key: "Ctrl+R / F5", what: qsTr("Rescan devices now") },
                            { key: "Drag & drop", what: qsTr("Send files to a device") },
                            { key: "?", what: qsTr("This list") },
                            { key: "Ctrl+Q", what: qsTr("Quit") }
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
            function onReceivedArrival(fileName) {
                tray.announceArrival(fileName)
                // With the window hidden, a toast nobody can see is no use.
                if (win.visible)
                    toast.show(qsTr("Received %1").arg(fileName))
            }
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
        // One scan at launch, then the periodic one; the backend also rescans
        // as soon as a peer says it came online, so this is not the only way
        // the list ever changes.
        backend.scanPeers()
        backend.startRescanCycle()
    }
}
