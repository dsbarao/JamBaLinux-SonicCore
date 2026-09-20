# Changelog

## 0.2.0 — unreleased

- rebrand the project and desktop presentation as JamBaLinux SonicCore;
- preserve installed CLI, service, Plasma, D-Bus, and cache identifiers for
  compatibility; document future migration in `docs/rebranding.md`;
- retain hardware/protocol names and research evidence, and document the
  initial host-side audio platform scope.

- add a persistent, host-side customizable 10-band equalizer profile with the
  confirmed QuantumENGINE frequencies and -12 dB to +12 dB range; expose it
  in the CLI and Plasma widget without USB/HID writes or PipeWire routing.

- control synchronized, Logo-only, or Ring-only solid colors from the widget;
- replace the fixed swatches with an embedded HSV wheel and saturation/value
  field, accepting any strictly validated `#RRGGBB` color;
- add complete-profile Breathing, Solid, Wave, and Glitch effects with 0.5x,
  1x, 1.5x, and 2x speeds;
- configure five independent colors per Logo and Ring zone while preserving
  both complete profiles on every hardware write;
- preserve both complete five-segment zone profiles whenever either zone is
  changed;
- migrate the previous synchronized color state without guessing unknown
  hardware values.

- add a disabled-by-default configuration, explicit gate, and read-only
  capability preflight for an experimental, open spatial/binaural audio
  foundation (`soniccore spatial`); it performs no PipeWire connection,
  routing, default-sink change, or DSP processing, and the widget only
  displays its state without any control that can activate it.

## 0.1.1 — 2026-09-17

- reserve enough popup height to display all solid-color controls without
  manual resizing;
- debounce USB-C topology changes before updating charging state;
- keep `Carregando` or `Carregado` visible for the entire time the cable is
  confirmed present, including while the headset is powered off.
- reserve a fixed status-message row so color changes do not move or resize
  the popup while showing `Aplicando` and `Configuração aplicada`.

## 0.1.0 — 2026-09-17

First usable Linux release for the JBL Quantum 810:

- detect USB device `0ecb:2069` and inspect interfaces/endpoints;
- correlate Game, Chat, and microphone ALSA nodes;
- decode the kernel-exported HID report descriptor;
- monitor and interpret confirmed spontaneous Input Reports;
- read battery Feature Report `0x49` through an allowlisted read-only ioctl;
- emit human-readable and schema-1 JSON battery status;
- display battery percentage through a KDE desktop notification;
- include an optional KDE application launcher;
- include a Plasma 6 panel widget with event-driven updates and a one-minute
  fallback refresh;
- show a battery-shaped panel indicator with headphones, percentage, and
  green/yellow/red charge thresholds;
- detect the USB-C charging connection as `0ecb:206a` and show a cyan charging
  state with a lightning indicator in the Plasma widget;
- add strictly allowlisted USB controls for ambient mode, global lighting,
  full-profile solid-color presets, and sidetone;
- replace floating control menus with polished in-widget expandable sections
  and keep the panel icon compact by moving the percentage into the detail view;
- use the Quantum 810 artwork in the detail view while keeping a compact
  battery/headphone indicator in the panel;
- add a read-only user service that caches confirmed HID events and exposes
  current ambient, lighting, microphone, battery, Bluetooth, and Game/Chat
  state to the widget; successful allowlisted commands also update the cache;
- visualize the physical Game/Chat dial as a live Chat-to-Game balance bar in
  the Plasma widget;
- replace five-second widget polling with event-driven D-Bus refreshes from the
  state service, retaining a low-frequency one-minute fallback;
- track USB-C hotplug independently so charging remains accurate while the
  headset is powered off;
- include guarded user-level installation and removal helpers;
- document controlled QuantumENGINE/USBPcap protocol findings.

There is no firmware, reset, interface-claiming, raw-report, or arbitrary
command path in this release. HID writes are limited to reviewed reports and
fixed values in the built-in allowlist.
