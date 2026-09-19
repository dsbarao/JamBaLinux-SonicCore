# JamBaLinux SonicCore v0.3 plan

Goal: turn the Plasma widget into a reliable real-time controller while keeping
all USB writes strictly allowlisted.

## 1. Read and expose current device state

- [x] Test read-only GET_FEATURE snapshots for ambient mode (`0x46`), global
  lighting (`0x4b`), and sidetone (`0x5d`). The device returned the battery
  report instead, so these controls cannot be queried as initial snapshots.
- [x] Maintain control state from confirmed HID input events and successful
  allowlisted commands, using `unknown` until evidence exists.
- [x] Expose cached confirmed state through `status --format json`.
- [x] Highlight confirmed selections in the Plasma widget.
- Add confirmed values to the versioned `status --format json` output.
- Show the current selection in the Plasma widget.
- Preserve `unknown` rather than guessing when a state cannot be read.

## 2. React to hardware and USB events

- [x] Observe confirmed Input Reports for headset power, ambient mode, microphone,
  lighting, battery, and Game/Chat balance.
- [x] Detect dongle and USB-C hotplug events.
- [x] Update the widget immediately instead of depending only on polling.

## 3. Introduce a single local state service

- [x] Own the hidraw event stream in one process.
- [x] Expose a small local, versioned API for the widget and CLI.
- Serialize writes so the widget, future desktop application, and OpenRGB do
  not contend for the HID interface.
- [x] Keep firmware, arbitrary reports, and unconfirmed writes out of scope.

## 4. Add safe lighting primitives

- [x] Generate complete Logo and Ring profiles from fixed presets.
- [x] Implement global on/off and synchronized full-profile solid colors.
- [x] Never overwrite one segment without replacing the complete profile.
- Use the same library as the future OpenRGB integration.

## 5. Prepare OpenRGB integration

- Model Logo and Ring as OpenRGB zones.
- Map the confirmed effects and speed values.
- Prototype as a separate plugin, then evaluate an upstream OpenRGB device
  controller once the protocol and hotplug behavior are stable.

## Acceptance criteria for step 1

- A read-only command reports battery, charging, ambient mode, lighting, and
  sidetone in machine-readable JSON.
- Values are confirmed against at least two controlled changes per feature.
- The widget highlights only confirmed current states.
