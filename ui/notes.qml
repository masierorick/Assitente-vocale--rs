import QtQuick 6.0
import QtQuick.Controls 6.0
import QtCore


Window {
    id: appWindow
    visible: true
    flags: Qt.FramelessWindowHint | Qt.Window
    color: "transparent"

    Settings { //Usa QtCore.Settings
        id: windowSettings
        location: typeof settingsPath !== "undefined" && settingsPath.length > 0
                  ? settingsPath
                  : StandardPaths.writableLocation(StandardPaths.ConfigLocation) + "/assistente-rs/settings.conf"
        category: "NotesWindow"    // ← chiavi salvate sotto [NotesWindow]
        property int savedX: 200
        property int savedY: 200
        property int savedWidth: 200
        property int savedHeight: 200
    }

    x: windowSettings.savedX
    y: windowSettings.savedY
    width: Math.max(testo.contentWidth + 20, 150)  // Larghezza minima 150
    height: Math.max(testo.contentHeight + 20, 100)  // Altezza minima 100

    // Animazioni
    Behavior on width { NumberAnimation { duration: 500; easing.type: Easing.InOutQuad } }
    Behavior on height { NumberAnimation { duration: 500; easing.type: Easing.InOutQuad } }

    Component.onCompleted: {
        width = windowSettings.savedWidth
        height = windowSettings.savedHeight
    }

    onXChanged: windowSettings.savedX = x
    onYChanged: windowSettings.savedY = y

    Rectangle {
        id: rectangle
        color: "#80000000"  // Semitrasparente
        radius: 10
        anchors.fill: parent
        anchors.margins: 10

        Flickable {
            id: flickable
            anchors.fill: parent
            anchors.margins: 10
            contentWidth: testo.width
            contentHeight: testo.height
            clip: true

            TextArea {
                objectName: "testo"
                id: testo
                text: processManager.testo
                color: "white"
                font.bold: true
                font.pointSize: 12
                wrapMode: Text.Wrap
                width: appWindow.width * 0.9
                readOnly: true
                selectByMouse: true
                background: Rectangle {
                    color: "transparent"
                }
                onTextChanged: {
                    flickable.contentY = flickable.contentHeight - flickable.height;
                }

                // Menu contestuale
                Menu {
                    id: contextMenu
                    MenuItem {
                        text: "Seleziona tutto"
                        onTriggered: testo.selectAll()
                    }
                    MenuItem {
                        text: "Copia"
                        onTriggered: testo.copy()
                    }
                    MenuItem {
                        text: "Sposta"
                        onTriggered: mouseArea.moveMode = true;  // Attiva la modalità di spostamento
                    }
                    MenuItem {
                        text: "Chiudi"
                        onTriggered: appWindow.close()
                    }
                }
            }

            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AlwaysOn }

        }
    }

    // MouseArea legato direttamente alla finestra
    MouseArea {
        id: mouseArea
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        hoverEnabled: true
        propagateComposedEvents: true
        property int edgeMargin: 10
        property bool moveMode: false

        function updateCursorShape(mouseX, mouseY) {
            if (mouseX < edgeMargin && mouseY < edgeMargin)
                mouseArea.cursorShape = Qt.SizeFDiagCursor;  // Angolo in alto a sinistra
            else if (mouseX > width - edgeMargin && mouseY > height - edgeMargin)
                mouseArea.cursorShape = Qt.SizeFDiagCursor;  // Angolo in basso a destra
            else if (mouseX < edgeMargin && mouseY > height - edgeMargin)
                mouseArea.cursorShape = Qt.SizeBDiagCursor;  // Angolo in basso a sinistra
            else if (mouseX > width - edgeMargin && mouseY < edgeMargin)
                mouseArea.cursorShape = Qt.SizeBDiagCursor;  // Angolo in alto a destra
            else if (mouseX < edgeMargin)
                mouseArea.cursorShape = Qt.SizeHorCursor;  // Bordo sinistro
            else if (mouseX > width - edgeMargin)
                mouseArea.cursorShape = Qt.SizeHorCursor;  // Bordo destro
            else if (mouseY < edgeMargin)
                mouseArea.cursorShape = Qt.SizeVerCursor;  // Bordo superiore
            else if (mouseY > height - edgeMargin)
                mouseArea.cursorShape = Qt.SizeVerCursor;  // Bordo inferiore
            else
                mouseArea.cursorShape = Qt.ArrowCursor;  // Nessun bordo
        }

        onPositionChanged: (mouse) => {
            updateCursorShape(mouse.x, mouse.y);
        }

        onPressed: (mouse) => {

             // --------------------------------------------------
             // PRIMA controlla se siamo sul bordo della finestra
             // --------------------------------------------------

              if (mouse.button === Qt.LeftButton) {

                  if (mouse.x < edgeMargin && mouse.y < edgeMargin) {
                      appWindow.startSystemResize(Qt.TopLeftCorner);
                      return;
                  }

                  if (mouse.x > width - edgeMargin && mouse.y < edgeMargin) {
                      appWindow.startSystemResize(Qt.TopRightCorner);
                      return;
                  }

                  if (mouse.x < edgeMargin && mouse.y > height - edgeMargin) {
                       appWindow.startSystemResize(Qt.BottomLeftCorner);
                       return;
                  }

                  if (mouse.x > width - edgeMargin && mouse.y > height - edgeMargin) {
                       appWindow.startSystemResize(Qt.BottomRightCorner);
                       return;
                  }

                  if (mouse.x < edgeMargin) {
                     appWindow.startSystemResize(Qt.LeftEdge);
                     return;
                  }

                  if (mouse.x > width - edgeMargin) {
                      appWindow.startSystemResize(Qt.RightEdge);
                      return;
                  }

                  if (mouse.y < edgeMargin) {
                      appWindow.startSystemResize(Qt.TopEdge);
                      return;
                  }

                  if (mouse.y > height - edgeMargin) {
                     appWindow.startSystemResize(Qt.BottomEdge);
                     return;
                  }
              }

               // --------------------------------------------------
               // SOLO DOPO controlla la TextArea
               // --------------------------------------------------

              var mappedPoint = mapToItem(testo, mouse.x, mouse.y);

              if (!moveMode && testo.contains(mappedPoint)) {

                 if (mouse.button === Qt.RightButton) {
                      contextMenu.popup();
                 } else {
                      mouse.accepted = false;
                 }

                 return;
              }

              // --------------------------------------------------
               // Spostamento finestra
              // --------------------------------------------------

              if (moveMode && mouse.button === Qt.LeftButton) {
                 appWindow.startSystemMove();
                 moveMode = false;
                 return;
              }

              // --------------------------------------------------
              // Menu contestuale
              // --------------------------------------------------

              if (mouse.button === Qt.RightButton)
                     contextMenu.popup();
              }


        onWheel: function(wheel) {
            if (wheel.modifiers & Qt.ControlModifier) {
                let delta = wheel.angleDelta.y / 120;
                appWindow.width += delta * 30;
                appWindow.height += delta * 30;
                windowSettings.savedWidth = appWindow.width;
                windowSettings.savedHeight = appWindow.height;

                if (appWindow.width < 150) appWindow.width = 150;
                if (appWindow.height < 100) appWindow.height = 100;
            } else {
                wheel.accepted = false;
            }
        }
    }

    Component.onDestruction: {
        windowSettings.savedX = appWindow.x;
        windowSettings.savedY = appWindow.y;
        windowSettings.savedWidth = appWindow.width;
        windowSettings.savedHeight = appWindow.height;
    }
}








