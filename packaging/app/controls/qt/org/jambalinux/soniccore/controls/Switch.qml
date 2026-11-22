// Qt's Switch whose label follows the Kirigami text color, like Plasma's.
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

QQC2.Switch {
    palette.windowText: Kirigami.Theme.textColor
    palette.text: Kirigami.Theme.textColor
}
