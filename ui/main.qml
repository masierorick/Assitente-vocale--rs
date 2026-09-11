import QtQuick 6.0
import QtQuick.Controls 6.0
import QtCore

ApplicationWindow  {
    id: main
    visible: true
    width: 220
    height: 220
    minimumWidth: 150
    minimumHeight: animation.height
    flags: Qt.FramelessWindowHint | Qt.Window | (animationManager.alwaysOnTop ? Qt.WindowStaysOnTopHint : 0)
    color: "transparent"
    property string textColor: "white"
    property string nomebot: configData.botname
    // "python" o "rust" — impostata dal backend come context property (vedi sotto)
    property string appVariant: typeof configData.variant !== "undefined" ? configData.variant : "python"
    property color accentColor: appVariant === "rust" ? "#CE7B32" : "#3776AB"

    Settings {
        id: settings
        location: typeof settingsPath !== "undefined" && settingsPath.length > 0
                  ? settingsPath
                  : StandardPaths.writableLocation(StandardPaths.ConfigLocation) + "/assistente-rs/settings.conf"
        category: "MainWindow"    // ← chiavi salvate sotto [MainWindow]
        property alias x : main.x
        property alias y : main.y
        property alias width : main.width
        property alias height : main.height

    }

    onWidthChanged: {
        settings.width = main.width;
    }
    onHeightChanged: {
        settings.height = main.height;
    }
    onXChanged: {
        settings.x = main.x;
    }
    onYChanged: {
        settings.y = main.y;
    }

    Connections {
        target: animationManager
        function onColorChanged(color) { textColor = color }
    }

    Timer {
        id: settingsCheckTimer
        interval: 200 // intervallo in millisecondi (1 secondo)
        running: true
        repeat: true
        onTriggered: {
            // Avvio processo di controllo se il botname è attivo e relativo cambio di colore
            animationManager.checkColor()
        }
    }

    // Menu contestuale
    Menu {
       id: contextMenu

       MenuItem {
          text: "Layout singolo"
          onTriggered: animationManager.loadWindow()
       }

       MenuItem {
          text: "Sempre in primo piano"
          checkable: true
          checked: animationManager.alwaysOnTop
          onTriggered: animationManager.setAlwaysOnTop(!animationManager.alwaysOnTop)
       }


    }

    Rectangle {
        id: animazione
        width: parent.width
        height: parent.height
        color: "transparent"

        AnimatedImage {
            id: animation
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
                    anchors.top: animazione.top
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

    MouseArea {
      anchors.fill: parent
      acceptedButtons: Qt.LeftButton | Qt.RightButton
      drag.target: parent
      property int edgeMargin: 10
      property bool moveMode: false

      onWheel: function(wheel) {
                let delta = wheel.angleDelta.y / 120; // 120 è il valore tipico per una rotazione del mouse wheel
                main.width += delta * 10; // Cambia la larghezza
                main.height += delta * 10; // Cambia l'altezza


                // Assicuriamoci che la finestra non diventi troppo piccola
                if (main.width < main.minimumWidth) main.width = main.minimumWidth;
                if (main.height < main.minimumHeight) main.height = main.minimumHeight;
            }
      onPressed: (mouse)=> {
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

          if (mouse.button == Qt.LeftButton)
                main.startSystemMove();
            else
             if (mouse.button == Qt.RightButton) {
                animationManager.stop_process()
                main.close();
           }
        }
     }



}
