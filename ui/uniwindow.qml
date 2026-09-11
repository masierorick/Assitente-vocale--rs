import QtQuick 6.0
import QtQuick.Controls 6.0
import QtCore

ApplicationWindow {
    id: uniwindow
    width: unisettings.width
    height: unisettings.height
    visible: true
    flags: Qt.FramelessWindowHint | Qt.Window | (animationManager.alwaysOnTop ? Qt.WindowStaysOnTopHint : 0)
    color: "transparent"
    minimumWidth: 350
    minimumHeight: animation.height

    property string textColor: "white"
    property string nomebot: configData.botname

    // "python" o "rust" — impostata dal backend come context property (vedi sotto)
    property string appVariant: typeof configData.variant !== "undefined" ? configData.variant : "python"
    property color accentColor: appVariant === "rust" ? "#CE7B32" : "#3776AB"

    Settings {
        id: unisettings
        location: typeof settingsPath !== "undefined" && settingsPath.length > 0
                  ? settingsPath
                  : StandardPaths.writableLocation(StandardPaths.ConfigLocation) + "/assistente-rs/settings.conf"
        category: "UniWindow"      // ← chiavi salvate sotto [UniWindow]
        property alias x : uniwindow.x
        property alias y : uniwindow.y
        property alias width : uniwindow.width
        property alias height : uniwindow.height
        onWidthChanged: {
            unisettings.width = uniwindow.width;
        }
        onHeightChanged: {
            unisettings.height = uniwindow.height;
        }
    }


    Timer {
        id: settingsCheckTimer
        interval: 200 // era 1000
        running: true
        repeat: true
        onTriggered: {
            // Avvio processo di controllo
            animationManager.checkColor();
        }
    }


    // Rectangle esterno con angoli arrotondati
    Rectangle {
        width: parent.width
        height: parent.height
        radius: 10
        color: "#80000000"
        border.color: accentColor
        border.width: 2

        // Usa Row per disporre le due sezioni affiancate
        Row {
            anchors.fill: parent
            spacing: 0

            // Prima parte: contenuto di listacom.qml sulla sinistra
            Rectangle {
                id: listacom
                width: parent.width * 0.7  // occupa il 70% della larghezza
                height: parent.height
                color: "transparent"
                anchors.margins: 10

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
                        width: parent.width
                        height: 30
                        color: "#303030"
                        radius: 5
                        border.color: "#505050"

                        TextField {
                            id: commandInput
                            width: parent.width -10
                            height: parent.height
                            //anchors.fill:parent
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
            }

            // Seconda parte: contenuto principale a destra
            Rectangle {
                id: animazione
                width: parent.width * 0.3  // occupa il 30% della larghezza
                height: parent.height
                color: "transparent"

                AnimatedImage {
                    id: animation
                    //anchors.fill: parent
                    anchors.margins: 10
                    anchors.centerIn: parent
                    source: "breath_round.gif"
                    height: 100
                    width: 100
                    smooth: false
                    cache: true



                Text {
                        objectName: "botname"
                        id: botname
                        font.family: "Space Age"
                        anchors.fill: parent
                        //anchors.margins: 15
                        font.bold: true
                        font.pointSize: 50
                        minimumPointSize: 5
                        fontSizeMode: Text.Fit
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        color: textColor
                        text: nomebot

                  }
                }

                // Etichetta variante (PY / RS) in basso a destra dell'animazione
                Text {
                    text: appVariant === "rust" ? "RS" : "PY"
                    color: accentColor
                    font.bold: true
                    font.pixelSize: 10
                    anchors.bottom: parent.bottom
                    anchors.right: parent.right
                    anchors.margins: 4
                }

                Button { // attiva il menu di configurazione
                    id: configButton
                    width: 18
                    height: 18
                    anchors.margins: 8
                    anchors.top: parent.top
                    anchors.right: parent.right
                    flat: true  // Rende il button senza bordi, se lo desideri
                    background: Rectangle {
                        color: "transparent"  // Rende lo sfondo trasparente, se lo desideri
                    }
                    contentItem: Image {
                        source: "settings.png"   // stesso path che funziona per il gif
                        fillMode: Image.PreserveAspectFit
                        anchors.centerIn: parent
                    }

                }


            }
        }


    }


    // Menu contestuale
    Menu {
        id: contextMenu

        MenuItem {
            text: "Layout separato"
            onTriggered: animationManager.loadWindow()
        }

        MenuItem {
            text: "Sempre in primo piano"
            checkable: true
            checked: animationManager.alwaysOnTop
            onTriggered: animationManager.setAlwaysOnTop(!animationManager.alwaysOnTop)
        }

    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        hoverEnabled: true
        drag.target: parent
        property int edgeMargin: 10
        property bool moveMode: false


        onWheel: function(wheel) {
             let mousePos = mapToItem(flickable, wheel.x, wheel.y);
             if (mousePos.x >= 0 && mousePos.y >= 0 && mousePos.x < flickable.width && mousePos.y < flickable.height) {
                    // L'evento è dentro la Flickable, lascia che scorra il testo
                 wheel.accepted = false;
                 return;
            }

            let delta = wheel.angleDelta.y / 120; // 120 è il valore tipico per una rotazione del mouse wheel
            uniwindow.width += delta * 10; // Cambia la larghezza
            uniwindow.height += delta * 10; // Cambia l'altezza

            // Assicuriamoci che la finestra non diventi troppo piccola
            if (uniwindow.width < 300) uniwindow.width = 300;
            if (uniwindow.height < 110) uniwindow.height = 110;
        }


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
            var mappedPoint = mapToItem(configButton, mouse.x,mouse.y);
            if (!moveMode && configButton.contains(mappedPoint)) {
                if (mouse.button == Qt.LeftButton) {
                    contextMenu.popup();
                }
                else {
                    mouse.accepted = false;
                    return;
                }
            }



            //gestione ridimensionamento
            if (mouse.button == Qt.LeftButton) {
                if (mouseX < edgeMargin && mouseY < edgeMargin)
                    uniwindow.startSystemResize(Qt.TopLeftCorner);
                else if (mouseX > width - edgeMargin && mouseY > height - edgeMargin)
                    uniwindow.startSystemResize(Qt.BottomRightCorner);
                else if (mouseX < edgeMargin && mouseY > height - edgeMargin)
                    uniwindow.startSystemResize(Qt.BottomLeftCorner);
                else if (mouseX > width - edgeMargin && mouseY < edgeMargin)
                    uniwindow.startSystemResize(Qt.TopRightCorner);
                else if (mouseX < edgeMargin)
                    uniwindow.startSystemResize(Qt.LeftEdge);
                else if (mouseX > width - edgeMargin)
                    uniwindow.startSystemResize(Qt.RightEdge);
                else if (mouseY < edgeMargin)
                    uniwindow.startSystemResize(Qt.TopEdge);
                else if (mouseY > height - edgeMargin)
                    uniwindow.startSystemResize(Qt.BottomEdge);
                else
                    uniwindow.startSystemMove();
            }
            else
                if (mouse.button == Qt.RightButton) {
                      Qt.callLater(function() { animationManager.stop_process(); });
                }
        }
    }

    Connections {
        target: animationManager
        function onNewOutput(msg) {
            testo.text += msg + "\n";
        }
        function onColorChanged(color) {
            textColor = color;
        }
    }



}
