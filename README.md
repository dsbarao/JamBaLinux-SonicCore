# JamBaLinux SonicCore

Open-source gaming headset control and audio platform for Linux.

**First supported device: JBL Quantum 810 Wireless.**

JamBaLinux SonicCore began with safe device detection and now includes
HID/USB integration, state monitoring, lighting, and strictly allowlisted
headset controls. Its independent identity leaves room for additional devices
while keeping host-side PipeWire audio processing separate from device
protocol controls.

> JamBaLinux SonicCore is an independent open-source project and is not affiliated with, endorsed by, or sponsored by Harman International or JBL. JBL, Quantum, and related product names are trademarks of their respective owners.

Technical identifiers use `jambalinux-soniccore`: the CLI is `soniccore`, the
daemon is `soniccore-daemon`, and the service is `jambalinux-soniccore.service`.
See [rebranding](docs/rebranding.md).

## Current status

Usable v0.1 controller and Plasma widget. On the initial CachyOS host, the dongle was detected as
`0ecb:2069` (`Harman International Inc`, `JBL Quantum810 Wireless`). It exposes
five USB Audio-class interfaces and one HID-class interface. See
[`docs/inventory/initial-cachyos.md`](docs/inventory/initial-cachyos.md).

Passive experiments have confirmed Input Reports for headset presence,
noise-control mode, Bluetooth state, microphone mute, and the 15-position
Game/Chat dial. Battery percentage and selected QuantumENGINE controls have
been confirmed through controlled USBPcap comparisons. The evidence ledger
links every conclusion to its experiment.

## Safety boundary

The repository permits passive discovery, allowlisted status reads, and only
the confirmed fixed-value controls documented under `docs/protocol/`:

- read Linux `sysfs` and udev metadata;
- inspect descriptors already exported by the kernel;
- enumerate ALSA/PipeWire nodes when accessible;
- query only battery Feature Report `0x49` through Linux `HIDIOCGFEATURE`;
- monitor spontaneous input through a read-only hidraw handle;
- permit only built-in ambient, lighting, solid-color, and sidetone Feature
  Reports whose fields and values were confirmed from controlled captures;
- accept strict `#RRGGBB` colors only through the confirmed complete-profile
  generator;
- reject raw reports, malformed RGB input, resets, driver detach, firmware
  operations, and guessed commands.

Any future active experiment must first be documented with provenance,
expected bytes, rollback/risk analysis, and explicit opt-in.

## Quick start

```bash
python3 tools/detect_quantum.py
./tools/inventory.sh
```

Both tools are read-only. `inventory.sh` prints to stdout; redirect it into the
ignored `work/` directory if a local snapshot is wanted.

### Rust CLI prerequisite on CachyOS

The `soniccore` command does not exist until the project has been compiled
and installed. On CachyOS, install `rustup` from an interactive terminal, then
select the stable toolchain:

```bash
sudo pacman -S --needed rustup
rustup default stable
rustc --version
cargo --version
```

Run the Rust CLI directly from the repository:

```bash
cargo check
cargo run -- scan
cargo run -- inspect
cargo run -- hid-descriptor
cargo run -- monitor --dry-run
cargo run -- status --dry-run
cargo run -- status --format json
cargo run -- notify --dry-run
cargo run -- export --format json
```

Only after that succeeds, optionally install the command for the current user:

```bash
cargo install --path .
soniccore scan
```

After changing the source, replace an older installed development build with:

```bash
cargo install --path . --force
soniccore inspect
```

If Cargo reports a successful install but Fish cannot find the command, add
Cargo's user binary directory to Fish's persistent path and start a new shell:

```fish
fish_add_path $HOME/.cargo/bin
soniccore scan
```

For a Bash session already open, update that session with:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
soniccore scan
```

`inspect` reports the full standard USB interface/endpoint topology from the
descriptor blob already exported by the kernel and correlates matching ALSA
cards and PCM nodes. `export --format json` emits the same data as structured,
version-friendly JSON.

`hid-descriptor` passively decodes the HID report descriptor already exported
by the kernel. It reports usage pages, Report IDs, directions, and sizes; it
does not open a hidraw device or send a HID request.

`monitor --dry-run` resolves the matching hidraw node by HID identity without
opening it. `monitor` uses a read-only file handle and prints only spontaneous
Input Reports until interrupted with Ctrl-C. It contains no write path and
does not issue HID ioctls or USB control requests. Confirmed report meanings
are appended as annotations while the original bytes remain visible.

`status --dry-run` validates the matched character device without opening it.
`status` opens hidraw read-only and performs only the allowlisted
`HIDIOCGFEATURE` read for Report `0x49`, then validates and prints battery
percentage. Hardware validation returned `49 3c` (60%). The `status` command
contains no SET_FEATURE or output-report path; confirmed writes are isolated
in `set`.

`status --format json` performs the same single allowlisted read and emits a
versioned object suitable for scripts and desktop widgets. It can be combined
with `--dry-run`; in that mode the battery fields are `null` and the device is
not opened.

`notify` performs the same single battery read and displays it through the
desktop notification service (`notify-send`). At 20% or below the notification
uses critical urgency. `notify --dry-run` does not open the device or display a
notification.

`show` opens a visible KDE dialog with the battery percentage. The application
launcher uses this command so clicking it produces a window instead of a
transient notification.

An optional KDE application launcher is included. Install it for the current
user after installing the CLI:

```bash
install -Dm644 packaging/kde/jambalinux-soniccore.desktop \
  "$HOME/.local/share/applications/jambalinux-soniccore.desktop"
```

It then appears in the application menu as **JamBaLinux SonicCore**.

### Plasma 6 panel widget

The repository also includes a native Plasma 6 widget with a compact panel
indicator. It receives state-change signals from the user daemon and retains a
low-frequency fallback refresh. While the USB-C charging connection
(`0ecb:206a`) is present, the widget changes to cyan and displays a lightning
indicator.

The CLI, widget, launcher, and user service can be validated and installed
together with the idempotent helper. `--check` makes no changes:

```bash
tools/install-user.sh --check
tools/install-user.sh
```

The helper does not install the privileged udev rule and does not restart the
Plasma shell.

To preview exactly what the user-level uninstaller would remove, run:

```bash
tools/uninstall-user.sh --check
```

Removal requires the explicit `--confirm` flag. It preserves the repository,
captures, documentation, and the system-wide udev rule:

```bash
tools/uninstall-user.sh --confirm
```

For manual widget installation, use:

```bash
kpackagetool6 --type Plasma/Applet --install \
  packaging/plasma/org.jambalinux.soniccore
```

For later development updates, use:

```bash
kpackagetool6 --type Plasma/Applet --upgrade \
  packaging/plasma/org.jambalinux.soniccore
```

Then enter Plasma edit mode, choose **Add Widgets**, search for
**JamBaLinux SonicCore**, and drag it to the panel. The widget invokes only
allowlisted `soniccore` commands. Its popup includes expandable controls
for ambient mode (off/ANC/TalkThru), global lighting (on/off), an HSV color
picker with five segments per zone, four effects, four animation speeds, and
independent or synchronized Logo and Ring profiles, plus hardware sidetone
(off/low/medium/high), and a 10-band host-side equalizer. The hardware controls
send only confirmed Feature Reports documented under `docs/protocol/`; raw
reports and malformed RGB values are rejected by the CLI parser. The equalizer
does not send a USB or HID report.

The installation helper also installs and starts a persistent user service for
the equalizer. There is no activation button: the service waits for the
confirmed Quantum Game output and automatically processes only playback
streams that were already routed there. Before moving a stream, it persists
its `pactl` index and restore identifier, plus the name and `pactl` index of its
original destination, so the route can be restored. Chat playback and capture
sources, including the microphone, remain outside the chain. The virtual
equalizer sink is never selected as the default; if it appears as the default,
the service restores the recorded non-virtual default.

Band changes and **Zerar bandas** issue four `Props` updates to the existing
PipeWire filter, separated by three nominal 10 ms intervals. The resulting
30 ms ramp excludes command and scheduling overhead. Real-system validation on
2026-09-19 measured the live audio effect and about 100 ms end-to-end command
time while preserving the service PID and filter-node identity. The update path
does not restart the service or deliberately recreate the filter node.
`soniccore equalizer status` and the widget report actionable errors when the
service, Game output, route, live DSP controls, or default-sink guard is not
healthy. See
[`docs/protocol/audio-processing.md`](docs/protocol/audio-processing.md) for the
architecture, acceptance evidence, and measurement boundary.

### Experimental open spatial/binaural foundation (disabled by default)

`soniccore spatial` stores an explicit, disabled-by-default gate for a future
open spatial/binaural audio feature under
`$XDG_CONFIG_HOME/jambalinux-soniccore/spatial.json`:

```bash
soniccore spatial               # show the stored gate and mode
soniccore spatial preflight     # read-only capability check
soniccore spatial status --format json
soniccore spatial enable        # requires a passing preflight; still applies no processing
soniccore spatial disable
```

This is currently a foundation only: it validates and persists configuration
and runs a read-only preflight that reports whether a `pipewire` binary, a
PipeWire filter-chain module, and a user-provided open HRTF/binaural dataset
under `spatial/hrtf/` in the configuration directory are present. `enable`
refuses with an actionable error when any of these is missing. Even when the
gate is recorded as enabled, this build performs no PipeWire connection,
routing, default-sink change, or DSP processing; `status` always reports
`active: false`. The Plasma widget only displays this state and the preflight
result under an **Espacial** section; it has no control that can turn spatial
audio on. Only the open, vendor-neutral name `binaural-stereo` is used for the
one defined mode; vendor names such as DTS or Quantum Spatial are intentionally
never used.

For real-time state tracking, install and enable the user service:

```bash
install -Dm644 packaging/systemd/jambalinux-soniccore.service \
  "$HOME/.config/systemd/user/jambalinux-soniccore.service"
systemctl --user daemon-reload
systemctl --user enable --now jambalinux-soniccore.service
```

The service opens the confirmed Quantum 810 hidraw node read-only, records only
known Input Reports, and writes a versioned cache under `XDG_RUNTIME_DIR`.
Unknown state remains `null`; it is never guessed.

If the monitor reports permission denied, install the narrowly scoped udev
rule and reconnect the dongle:

```bash
sudo install -m 0644 packaging/udev/70-jambalinux-soniccore.rules /etc/udev/rules.d/70-jambalinux-soniccore.rules
sudo udevadm control --reload-rules
```

The rule matches only USB device `0ecb:2069`, HID interface `05`, and uses
systemd-logind's `uaccess` mechanism. It does not use world-writable mode,
create a privileged daemon, or grant access to unrelated hidraw devices.

## Repository layout

- `docs/protocol/`: evidence ledger and protocol hypotheses;
- `docs/reverse-engineering.md`: capture workflow and safety gates;
- `docs/windows-vm-capture.md`: controlled QuantumENGINE/USBPcap workflow;
- `docs/plan-v0.1.md`: milestone definition;
- `tools/`: passive host/device inventory utilities;
- `src/`: Linux CLI, HID parsing, allowlisted controls, and state service;
- `docs/rebranding.md`: project identity and technical identifiers;
- `captures/`: local capture staging; capture files are ignored by Git.

## Confirmed passive mappings

| Report | Values | Meaning |
|---|---|---|
| `0x02` | `00/01/02` | noise control off / ANC / TalkThru |
| `0x03` | `00/01/02` | Bluetooth disconnected / connected / pairing |
| `0x06` | `00/01` | microphone muted / active |
| `0x07` | `00/01` | lighting disabled / enabled |
| `0x08` | `00–64` | battery percentage (0–100 decimal) |
| `0x09` | `00/01` | headset off-disconnected / on-connected |
| `0x10` | `00–06, 08, 0a–10` | Game/Chat balance, normalized -7…+7 |
| `0x2f` | bit 1 | HID Telephony Phone Mute pulse |

QuantumENGINE startup returned Feature Report `0x49` with `0x5a` while the
dongle emitted Input Report `08 5a`. A later controlled lighting capture emitted
`08 55`, confirming the Input Report payload tracks the changing battery
percentage (90%, then 85%).

## Platform direction

The following is a conceptual direction, not a list of implemented modules:

```text
JamBaLinux SonicCore
├── Device Control
│   ├── Battery
│   ├── ANC / TalkThru
│   ├── Sidetone
│   └── HID controls
├── Lighting
│   └── RGB / effects
└── Audio
    ├── Game/Chat Mix
    ├── Equalizer
    ├── PipeWire DSP
    └── Spatial Audio
```

Device Control and solid RGB lighting already have confirmed implementations
for the first supported headset. Animation effects are documented research,
not selectable Linux controls. Game/Chat currently displays the physical dial
state. A persistent user service implements a customizable 10-band host-side
equalizer for streams already destined for Game; routing is automatic and
reversible, while Chat, microphone, and the system default route remain outside
the processing policy. Its current filters are a Linux approximation, not a
claim of QuantumENGINE parity. Real PipeWire/headset acceptance on 2026-09-19
covered browser continuity, measured gain changes, widget/profile/DSP reset,
Game-only isolation, default-route recovery, reversible routing, and a physical
dongle reconnect. Software Game/Chat mixing remain future work; open
spatial/binaural audio has only a disabled-by-default configuration, gate, and
capability preflight so far, with no processing implemented yet. This work
adds no device-side audio functionality and does not change USB/HID safety
boundaries.

## Scope

Linux is the development host. A Windows VM with QuantumENGINE,
USBPcap/Wireshark, and controlled USB passthrough is reserved for later
observation of known-good vendor traffic. No protocol claim should be treated
as fact until linked to repeatable evidence.

## Contributing

Do not submit firmware, proprietary binaries, personal identifiers, or raw
captures that may include unrelated USB traffic. Prefer small sanitized byte
sequences with timestamps removed and document hardware/firmware versions.

For collaboration, bug reports, or responsible disclosure of a safety concern,
open a GitHub issue or contact [jambalinux@gmail.com](mailto:jambalinux@gmail.com).

Licensed under the MIT License.
