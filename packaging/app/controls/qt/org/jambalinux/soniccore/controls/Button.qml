// Qt's Button, in the desktop's style: for GNOME, COSMIC, Cinnamon and others.
// The chosen option (checked) is tinted with the accent color, as Plasma's
// button shows it even when it is not checkable. The tint sits just above the
// style's background (and below the Hi-Fi metal key, which covers it).
import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

QQC2.Button {
    id: control

    Rectangle {
        z: -0.75
        anchors.fill: parent
        anchors.margins: 1
        radius: Kirigami.Units.cornerRadius
        visible: control.checked && !control.flat
        color: Qt.alpha(Kirigami.Theme.highlightColor, 0.3)
        border.width: 1
        border.color: Kirigami.Theme.highlightColor
    }
}
