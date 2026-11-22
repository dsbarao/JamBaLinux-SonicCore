// Qt's Label with Plasma's defaults: plain text and the Kirigami colors
// (Fusion would use the window palette and ignore the Hi-Fi engraving).
import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

QQC2.Label {
    color: Kirigami.Theme.textColor
    linkColor: Kirigami.Theme.linkColor
    textFormat: Text.PlainText
    verticalAlignment: lineCount > 1 ? Text.AlignTop : Text.AlignVCenter
}
