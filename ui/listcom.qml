import QtQuick 6.0
import QtQuick.Controls 6.0
import QtCore

Window {
    id:listcom
    visible: true
    flags: Qt.FramelessWindowHint | Qt.Window | (animationManager.alwaysOnTop ? Qt.WindowStaysOnTopHint : 0)
    color: "transparent"

    // "python" o "rust" — impostata dal backend come context property
    property string appVariant: typeof configData.variant !== "undefined" ? configData.variant : "python"
    property color accentColor: appVariant === "rust" ? "#CE7B32" : "#3776AB"


    Settings {
        id: windowSettings
        category: "ListcomWindow"  // ← chiavi salvate sotto [ListcomWindow]
        property  int savedX: 200
        property  int savedY: 200
        property int savedWidth : 400
        property int savedHeight : 200
    }

    x: windowSettings.savedX
    y: windowSettings.savedY
    width: 0 //windowSettings.savedWidth
    height: 0 //windowSettings.savedHeight

    // Animazione per la larghezza
    Behavior on width {
        NumberAnimation {
            duration: 500
            easing.type: Easing.InOutQuad
        }
    }

    // Animazione per l'altezza
    Behavior on height {
        NumberAnimation {
            duration: 500
            easing.type: Easing.InOutQuad
        }
    }

    // Animazione all'apertura della pagina
    Component.onCompleted: {
        width = windowSettings.savedWidth
        height = windowSettings.savedHeight
    }

    // Monitoraggio dei cambiamenti di posizione per salvarli
    onXChanged: windowSettings.savedX = x
    onYChanged: windowSettings.savedY = y
    onWidthChanged: {
        windowSettings.savedWidth = listcom.width;
    }
    onHeightChanged: {
        windowSettings.savedHeight = listcom.height;
    }

    Rectangle {
        id: rectangle
        color: "#80000000"
        radius: 10
        anchors.fill: parent
        anchors.margins: 10
        height: listcom.height - 30
        border.color: accentColor
        border.width: 2

        Flickable {
            id: flickable
            anchors {
                top: parent.top
                left: parent.left
                right: parent.right
                bottom: commandColumn.top
                margins: 10
            }

            contentWidth: testo.width
            contentHeight: testo.height
            clip: true

            Text {
                objectName: "testo"
                id: testo
                text: ""
                color: "white"
                width: parent.width
                wrapMode: Text.Wrap
                font.pixelSize: 12
                textFormat: Text.PlainText

                onTextChanged: {
                    flickable.contentY = flickable.contentHeight - flickable.height;
                }
            }
            ScrollBar.vertical: ScrollBar {
                policy: ScrollBar.AlwaysOn
            }
        }

        Column {
            id: commandColumn
            anchors {
                bottom: parent.bottom
                left: parent.left
                right: parent.right
                margins: 10
            }
            spacing: 5

            Rectangle {
                id: comando
                width: parent.width
                height: 30
                color: "#303030"
                radius: 5
                border.color: "#505050"

                TextField {
                    id: commandInput
                    width: parent.width - 10
                    height: parent.height
                    anchors.centerIn: parent
                    anchors.margins: 10
                    placeholderText: "Inserisci un comando..."
                    color: "black"
                    selectionColor: "#606060"
                    focus: true

                    onAccepted: {
                        if (text.trim().length > 0) {
                            testo.text += "> " + text + "\n";
                            animationManager.sendCommand(text);
                            text = "";
                        }
                    }
                }
            }
        }

        MouseArea {
            id: mouseArea
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            hoverEnabled: true

            property int edgeMargin: 10

            function updateCursorShape(mouseX, mouseY) {
                if (mouseX < edgeMargin && mouseY < edgeMargin)
                    mouseArea.cursorShape = Qt.SizeFDiagCursor;
                else if (mouseX > width - edgeMargin && mouseY > height - edgeMargin)
                    mouseArea.cursorShape = Qt.SizeFDiagCursor;
                else if (mouseX < edgeMargin && mouseY > height - edgeMargin)
                    mouseArea.cursorShape = Qt.SizeBDiagCursor;
                else if (mouseX > width - edgeMargin && mouseY < edgeMargin)
                    mouseArea.cursorShape = Qt.SizeBDiagCursor;
                else if (mouseX < edgeMargin)
                    mouseArea.cursorShape = Qt.SizeHorCursor;
                else if (mouseX > width - edgeMargin)
                    mouseArea.cursorShape = Qt.SizeHorCursor;
                else if (mouseY < edgeMargin)
                    mouseArea.cursorShape = Qt.SizeVerCursor;
                else if (mouseY > height - edgeMargin)
                    mouseArea.cursorShape = Qt.SizeVerCursor;
                else
                    mouseArea.cursorShape = Qt.ArrowCursor;
            }

            onPositionChanged: (mouse) => {
                updateCursorShape(mouse.x, mouse.y);
            }

            onPressed: (mouse) => {
                if (mouse.button == Qt.LeftButton) {
                    if (mouseX < edgeMargin && mouseY < edgeMargin)
                        listcom.startSystemResize(Qt.TopLeftCorner);
                    else if (mouseX > width - edgeMargin && mouseY > height - edgeMargin)
                        listcom.startSystemResize(Qt.BottomRightCorner);
                    else if (mouseX < edgeMargin && mouseY > height - edgeMargin)
                        listcom.startSystemResize(Qt.BottomLeftCorner);
                    else if (mouseX > width - edgeMargin && mouseY < edgeMargin)
                        listcom.startSystemResize(Qt.TopRightCorner);
                    else if (mouseX < edgeMargin)
                        listcom.startSystemResize(Qt.LeftEdge);
                    else if (mouseX > width - edgeMargin)
                        listcom.startSystemResize(Qt.RightEdge);
                    else if (mouseY < edgeMargin)
                        listcom.startSystemResize(Qt.TopEdge);
                    else if (mouseY > height - edgeMargin)
                        listcom.startSystemResize(Qt.BottomEdge);
                    else
                        listcom.startSystemMove();
                }
            }
        }
    }

    Connections {
        target: animationManager
        function onNewOutput(msg) {
            testo.text += msg + "\n";
        }
    }

    Component.onDestruction: {
        listcom.x = windowSettings.savedX;
        listcom.y = windowSettings.savedY;
        listcom.width = windowSettings.savedWidth;
        listcom.height = windowSettings.savedHeight;
    }
}


