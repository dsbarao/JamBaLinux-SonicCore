import QtQuick
import QtQuick.Layouts

import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.plasmoid

// The Plasma widget: a thin shell around SonicCore.qml, which the
// standalone app shares.
PlasmoidItem {
    id: widget

    SonicCore {
        id: core
        configuration: Plasmoid.configuration
        expanded: widget.expanded
        onToggleRequested: widget.expanded = !widget.expanded
    }

    Plasmoid.icon: "audio-headphones"
    Plasmoid.status: core.batteryPercent >= 0 ? PlasmaCore.Types.ActiveStatus : PlasmaCore.Types.PassiveStatus

    // Plasma creates representations in this file's context, where the
    // ids inside SonicCore.qml do not exist; a Loader creates them in
    // their own context and passes their size hints on to Plasma.
    compactRepresentation: Loader {
        sourceComponent: core.compactRepresentation
        Layout.minimumWidth: item ? item.Layout.minimumWidth : 0
        Layout.preferredWidth: item ? item.Layout.preferredWidth : 0
        Layout.minimumHeight: item ? item.Layout.minimumHeight : 0
        Layout.preferredHeight: item ? item.Layout.preferredHeight : 0
    }
    fullRepresentation: Loader {
        sourceComponent: core.fullRepresentation
        Layout.minimumWidth: item ? item.Layout.minimumWidth : 0
        Layout.preferredWidth: item ? item.Layout.preferredWidth : 0
        Layout.minimumHeight: item ? item.Layout.minimumHeight : 0
        Layout.preferredHeight: item ? item.Layout.preferredHeight : 0
    }
}
