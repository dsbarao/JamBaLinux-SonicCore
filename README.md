<p align="center">
  <img src="packaging/brand/jambalinux-soniccore-256.png" alt="" width="128">
  <br>
  <img src="packaging/brand/wordmark.png" alt="JamBaLinux Sonic Core" width="480">
</p>

# JamBaLinux Sonic Core

Open-source gaming headset control and audio platform for Linux.

**First supported device: JBL Quantum 810 Wireless.**

JamBaLinux Sonic Core began with safe device detection and now includes
HID/USB integration, state monitoring, lighting, and strictly allowlisted
headset controls. Its independent identity leaves room for additional devices
while keeping host-side PipeWire audio processing separate from device
protocol controls.

> JamBaLinux Sonic Core is an independent open-source project and is not affiliated with, endorsed by, or sponsored by Harman International or JBL. JBL, Quantum, and related product names are trademarks of their respective owners.

Technical identifiers use `jambalinux-soniccore`: the CLI is `soniccore`, the
daemon is `soniccore-daemon`, and the service is `jambalinux-soniccore.service`.
See [rebranding](docs/rebranding.md).

## Current status

Usable controller and desktop app for **any Linux desktop**: Plasma, GNOME,
COSMIC, Cinnamon, MATE and Xfce (see [Install](#install)). On the initial
CachyOS host, the dongle was detected as
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

<a id="install"></a>
### Install (any desktop)

Sonic Core has two parts:

- **The core** — the `soniccore` CLI, the user daemon, the equalizer and 3D
  PipeWire services. It needs PipeWire (with `pactl`), a systemd user session
  and Rust 1.88 or newer, and runs on every desktop.
- **The app** — the window with the battery in the tray, the equalizer, 3D
  sound, *Tocando agora* (media and volume) and the lights, with the Vivo,
  Hi-Fi and Leve themes. A small Qt launcher (`packaging/app/launcher`) opens
  the interface with its helpers built in, so it needs no Plasma library: on
  Plasma it uses Plasma's own controls, elsewhere Qt's in the desktop's style.
  It needs Qt 6.5 or newer and KDE's Kirigami 6.

Install the dependencies for your distribution, then run the installer from
the repository. `--check` changes nothing:

```bash
tools/install-user.sh --check
tools/install-user.sh
```

| Distribution | Dependencies |
| --- | --- |
| Arch, CachyOS, Manjaro (any desktop) | `sudo pacman -S --needed rustup base-devel qt6-base qt6-declarative qt6-svg qt6-wayland qt6-multimedia qt6-multimedia-ffmpeg kirigami qqc2-desktop-style libpulse` then `rustup default stable` |
| Fedora 41+ (Workstation/GNOME, KDE, Cinnamon spins) | `sudo dnf install cargo gcc-c++ make qt6-qtbase-devel qt6-qtdeclarative-devel qt6-qtsvg qt6-qtwayland qt6-qtmultimedia kf6-kirigami kf6-qqc2-desktop-style pulseaudio-utils` |
| Debian 13, Ubuntu 25.04+ | Qt 6 and Kirigami 6 from the distribution (`qt6-base-dev`, `qt6-declarative-dev`, the `qml6-module-*` packages for QtQuick Controls, Layouts, Effects, Shapes, QtCore, Qt.labs.platform, QtMultimedia and `org.kde.kirigami`), `build-essential`, `pulseaudio-utils`, and Rust from [rustup](https://rustup.rs) (the packaged cargo is older than 1.88). Package names can vary between releases. |
| **Ubuntu 24.04, Pop!_OS 24.04 (COSMIC), Linux Mint 22 (Cinnamon)** | Their Qt is 6.4, too old for the app: install the app as a **Flatpak** (below). `sudo apt install flatpak build-essential pulseaudio-utils`, Rust from [rustup](https://rustup.rs). |

On Ubuntu 24.04 and the distributions built on it, install the core and the
app as a Flatpak (the KDE runtime from Flathub brings Qt 6 and Kirigami, about
1 GB the first time):

```bash
tools/install-user.sh --flatpak
```

`tools/build-flatpak.sh` rebuilds only the Flatpak (`--bundle FILE` also
writes a single-file bundle to install elsewhere with
`flatpak install --user FILE`). The Flatpak reaches the host's `soniccore`
through `flatpak-spawn`.

The installer does not install the privileged udev rule (see above) and does
not restart the desktop.

**Rádio.** *Tocando agora* also plays internet radio: pick a style (Rock,
Heavy Metal, Synthwave, Anos 80, Lo-fi, MPB, …) and change stations with the
previous/next buttons; ♥ keeps a station, and the first six favorites become
memory keys. Stations come from [Radio Browser](https://www.radio-browser.info),
a free community catalog (only working stations, best voted first, Brazil
only with **BR**); the song title is read from the station's own stream. While
it is on, the radio is a media player of the desktop (MPRIS), so the
keyboard's media keys and the desktop's media controls work with it, and it
goes through the equalizer and 3D like any app. The app contacts the internet
only while the radio is on, and does not report "clicks" to the catalog.

**Desktop notes.** Plasma, COSMIC, Cinnamon, MATE and Xfce show the battery
in the tray. GNOME shows tray icons only with the *AppIndicator and
KStatusNotifierItem* extension (Ubuntu and Pop!_OS ship it); without a tray
the app is a normal window and is not started at login. Only Plasma (KWin)
lets the app keep its window position; elsewhere the size is remembered and
the desktop places the window. The *Papel de parede* background reads each
desktop's own wallpaper setting.

To preview exactly what the user-level uninstaller would remove, run:

```bash
tools/uninstall-user.sh --check
```

Removal requires the explicit `--confirm` flag. It preserves the repository,
captures, documentation, and the system-wide udev rule:

```bash
tools/uninstall-user.sh --confirm
```

#### Game volume fix (user-level ACP overlay)

On the JBL Quantum 810 Wireless, the upstream PipeWire profile-set
(`usb-gaming-headset-gamefirst.conf`) wires the **Game** sink volume to the
dongle's `PCM,1` control, which is the **Chat** hardware volume. The real Game
hardware volume, `PCM,0`, is never driven and keeps its last stored value
(observed at -23 dB), so the dongle sounds quiet and the extra gain ends up
being pushed digitally until it clips.

`tools/install-user.sh` therefore also installs a small user-level overlay
(`tools/acp-overlay.sh --install`): a profile-set and three mixer paths under
`~/.config/alsa-card-profile/mixer/`, plus a WirePlumber rule that selects that
profile-set only for the Quantum 810 card. With it, Game drives `PCM,0` and
Chat drives `PCM,1`; node, mapping, and profile names stay the same. Nothing is
written under `/usr`, no `sudo` is used, and udev, HID, VID/PID, and allowlists
are untouched.

The overlay takes effect when WirePlumber recreates the card. Reconnect the
dongle or run:

```bash
systemctl --user restart wireplumber
```

Check, verify, install, or remove only the overlay with:

```bash
tools/acp-overlay.sh --check
tools/acp-overlay.sh --verify
tools/acp-overlay.sh --install
tools/acp-overlay.sh --uninstall
```

`--verify` changes nothing and fails if a target file was edited outside
JamBaLinux; `tools/install-user.sh` runs it before installing anything, so an
edited overlay stops the installation before the CLI, app, or services are
touched. `tools/uninstall-user.sh --confirm` removes the overlay too. Files edited
outside JamBaLinux are kept and reported instead of deleted. After removal and
a WirePlumber restart, the upstream profile-set applies again. A draft report
for upstream is in
[`docs/bug-reports/quantum810-acp-game-volume.md`](docs/bug-reports/quantum810-acp-game-volume.md).

The app invokes only allowlisted `soniccore` commands. It includes controls
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

Band changes, predefined profile selection, and **Zerar bandas** ramp the
existing PipeWire filter through `Props` updates 22 ms apart (about one audio
quantum), moving no band more than 0.25 dB per step. A ramp uses at least four
and at most 48 updates, so a small change takes about 70 ms and a full-scale
reset about one second, excluding command and scheduling overhead. Real-system validation on
2026-09-19 measured the live audio effect and about 100 ms end-to-end command
time while preserving the service PID and filter-node identity. The update path
does not restart the service or deliberately recreate the filter node.
`soniccore equalizer status` and the app report actionable errors when the
service, Game output, route, live DSP controls, or default-sink guard is not
healthy. See
[`docs/protocol/audio-processing.md`](docs/protocol/audio-processing.md) for the
architecture, acceptance evidence, and measurement boundary.

#### Equalizer profiles

The 17 captured profiles are immutable factory presets. Named custom profiles
are user-owned data, stored next to the active bands in
`$XDG_CONFIG_HOME/jambalinux-soniccore/equalizer.json`:

```bash
soniccore equalizer preset fps                     # select a factory preset
soniccore equalizer profile create "Noite"         # save the current bands
soniccore equalizer profile list
soniccore equalizer profile list --format json
soniccore equalizer profile apply custom-4711-0
soniccore equalizer profile rename custom-4711-0 "Noite calma"
soniccore equalizer profile update custom-4711-0   # overwrite with active bands
soniccore equalizer profile delete custom-4711-0
```

The `profile` subcommands accept only custom profile IDs: factory presets
cannot be applied, updated, renamed, or deleted through them, and
`equalizer preset <id>` remains their dedicated selector. Deleting the selected
profile applies **Flat** in the same transaction. The app's equalizer card
offers the same operations through a profile selector plus **Criar perfil**,
**Atualizar**, **Renomear**, and **Excluir**, and reads its list from
`soniccore equalizer status --format json`, which also reports
`active_profile_id` and `custom_profiles`. An earlier single-profile
`equalizer.json` is migrated automatically, preserving a hand-tuned setting as a
custom profile named `Personalizado`.

Profiles are host-side DSP settings: none of these commands opens the headset,
sends a USB or HID report, or changes what is processed — only the confirmed
Quantum Game output is, while Chat, capture sources, and the microphone stay
outside the chain.

### Experimental open spatial/binaural processing (disabled by default)

`soniccore spatial` controls an explicit, disabled-by-default open
spatial/binaural audio processor. Its profile is stored under
`$XDG_CONFIG_HOME/jambalinux-soniccore/spatial.json`:

```bash
soniccore spatial               # show the stored gate and mode
soniccore spatial preflight     # read-only capability check
soniccore spatial status --format json
soniccore spatial mode binaural-stereo  # select the graph (mode `off` removes it)
soniccore spatial enable        # requires a passing preflight
soniccore spatial disable       # always allowed; keeps the graph in bypass
soniccore spatial run           # supervisor loop; started by the user service
soniccore spatial restore       # manual: move spatial streams to the equalizer
```

The read-only preflight validates the `pipewire` binary from `PATH`, filter-chain module,
and a user-provided 14-channel HRIR dataset (`manifest.json` plus `hrir.wav`)
under `spatial/hrtf/` in the configuration directory. `enable` refuses with an
actionable error when that preflight fails; it does not require the equalizer
to be running. The equalizer is the graph's only output, so the supervisor
creates the graph only once the equalizer sink exists and relinks it
automatically if the equalizer restarts.

The graph is owned by the `jambalinux-soniccore-spatial.service` user unit
(`soniccore spatial run`), installed and started by `tools/install-user.sh`.
It runs one PipeWire snapshot per pass (every second while idle). On stop,
systemd signals only the supervisor (`KillMode=mixed`): it unlinks the graph,
moves its streams to the equalizer, and then stops the graph process. Stop the
spatial service before the equalizer service, as the installer does.

While mode is `binaural-stereo` and readiness holds, the supervised 7.1
graph persists and is linked only through `Spatial -> EQ -> Quantum Game`; it
never becomes the default sink, and Chat and capture nodes remain excluded.
The enable gate crossfades its wet binaural and dry downmix controls in place.
Thus disabling reports **Desativado (bypass)** while retaining the selected
spatial sink and its streams; selecting mode `off` or losing readiness still
uses the safe stream-restoring teardown. The Plasma **Espacial** section
exposes the same gate and health state. Only the
open, vendor-neutral mode name `binaural-stereo` is used; vendor names or files
are intentionally never used. Technical hardware acceptance is recorded in
[`docs/protocol/validation_reports/spatial-hardware-validation.md`](docs/protocol/validation_reports/spatial-hardware-validation.md).

When `PIPEWIRE_MODULE_DIR` is unset or empty, module preflight checks the
standard `/usr/lib/pipewire-0.3` and `/usr/lib64/pipewire-0.3` directories,
plus the applicable Debian-style multiarch directory
(`/usr/lib/x86_64-linux-gnu/pipewire-0.3` or
`/usr/lib/aarch64-linux-gnu/pipewire-0.3`). When it is set, PipeWire treats it
as the complete module search path: set it to the directory or colon-separated
directory list containing `libpipewire-module-filter-chain.so`; standard paths
are not also checked. Configure it in the user systemd manager environment
(for example through `environment.d`), not only in an interactive shell, so
the preflight and spawned PipeWire graph see the same value. No files are
downloaded or installed automatically. The installation and removal helpers
locate the CLI under `${CARGO_HOME:-$HOME/.cargo}/bin`. With a custom
`CARGO_HOME`, `tools/install-user.sh` installs copies of the systemd units,
launcher, and app that point at that directory; paths containing spaces,
quotes, `\`, `$`, `` ` ``, `|`, or `%` are refused.

The persistence/bypass transition still needs an in-person auditory acceptance:
keep a browser video playing and toggle spatial ten times, confirming that it
never pauses and that the graph node identities remain unchanged.

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

### Battery history and lighting economy

The daemon records every change of battery percentage, charging, or lighting
state in `$XDG_STATE_HOME/jambalinux-soniccore/battery-history.jsonl`
(default `~/.local/state/…`), together with the effect, the mean LED
brightness, and the ambient mode (recorded only). Discharging intervals give a
measured drain per hour with the lights on and off and an estimate of the
time left, shown in the app once there is enough data.

The optional economy uses only the confirmed lighting on/off control:

```bash
soniccore power                         # settings, measured drain, time left
soniccore power set low-battery 20      # lights off at 20% or less, away from the charger
soniccore power set idle 10             # lights off after 10 minutes without sound
soniccore power set idle off
```

Lights come back on with the same colors and effect when charging (or 5
points above the threshold) or when sound returns. Switching them on during
an economy period is respected until the condition arms again. While the
economy keeps the lights off, OpenRGB writes are refused like under
`lighting-hold`. Ambient (ANC/TalkThru) is never changed. The app's
**Configurar → Bateria** page sets the same options.

### Other audio outputs

The equalizer, and the spatial graph behind it, can play into any audio output
of the PC, not only the Quantum 810's Game channel:

```bash
soniccore output --format json                 # setting, what it resolves to now, outputs
soniccore output set auto                      # follow the system default output (default)
soniccore output set alsa_output.pci-0000_0d_00.4.analog-stereo   # a fixed output
soniccore output set quantum-game              # always the Quantum 810's Game channel
```

With `auto`, picking another default output in Plasma's volume applet moves
the equalizer (and the spatial graph) there within about two seconds: the
service rebuilds its filter for the new output and points the restored
destinations of its streams at it. Any Quantum 810 output chosen as default,
Chat included, means its Game channel.

Each output remembers its own equalizer setting in `output-profiles.json`:
when the equalizer leaves an output it records the selected profile (or the
exact bands of unsaved manual settings), and when it arrives at an output
used before it ramps to that output's setting. A remembered profile is
selected by id, so later edits of a custom profile are kept; an output never
used before keeps the current setting.

`output set` saves the choice in `output.json` and restarts the equalizer
service, which rebuilds the filter for that output; the spatial graph relinks
by itself, and the live meter and audio path follow it. Streams played on the
chosen output are routed through the equalizer the same way Game streams are.
The Quantum's Chat channel is never offered, keeping the microphone and calls
isolated. Lighting, ANC, sidetone, and battery stay with the headset. The
app's **Configurações → Saída de áudio** page offers the same choice.

### Vivo theme backgrounds

With the app's **Vivo** theme, the **Fundo** menu also picks a
background: **Aurora** (soft light in the headset lights' colors, drifting
slowly), **Vidro** (the desktop wallpaper, blurred and dimmed, found with
`soniccore wallpaper --format json`), **Partículas** (glowing dots rising),
**Ondas** (glowing lines following the sound), or none; with an intensity and an
option to pulse with the bass from the live meter. Each light is drawn once
into a cached texture and only moved, scaled, and faded; every moving part of
the popup shares one 25 Hz beat instead of redrawing at the screen's refresh
rate. Particles and waves are fragment shaders (`contents/shaders/*.frag`,
compiled to `.qsb` by `tools/build-shaders.sh`; a test checks they are up to
date), drawn per pixel so they stay smooth at any size. Measured with sound playing, the whole open Vivo popup costs about 6% of
one CPU core (Aurora adds under 1%); closed, nothing runs.

### Live meter and audio path

Two read-only commands feed the app's Vivo and Hi-Fi themes; neither
changes routing nor touches the headset.

```bash
soniccore meter --format json --frames 25   # 32 spectrum bands + peak, 25 per second
soniccore audio-path --format json          # apps -> spatial -> EQ -> Quantum Game
```

`meter` records the monitor of the exact Quantum Game sink with a passive
`pw-record` capture and analyzes it with a 2048-point FFT (31 Hz to 16 kHz,
-70 to -10 dBFS mapped onto 0 to 1); silence skips the FFT. The app reaches
it through the session bus: `tools/install-user.sh` installs a D-Bus
activation file for `org.jambalinux.soniccore.Meter`, so `soniccore meter
--dbus` starts only when a frame is requested, stops capturing two seconds
after the last request, and exits after thirty idle seconds. `audio-path`
reads one `pw-dump` snapshot and reports each stage (present, running) and
each link of the Game path.

### OpenRGB plugin

OpenRGB 1.1 does not detect the Quantum 810. The optional plugin in
`openrgb-plugin/` registers it as an OpenRGB device, **JBL Quantum 810
Wireless**, with two zones (Logo and Ring, five LEDs each) and the modes
Direct, Breathing, Wave, Glitch (with speed), and Off. It never opens the
headset itself: every change runs the `soniccore` CLI, which keeps the
allowlisted reports, the lighting lock, and the status cache in one place.

```bash
tools/install-openrgb-plugin.sh --check
tools/install-openrgb-plugin.sh                 # clones the matching OpenRGB release headers
tools/install-openrgb-plugin.sh --openrgb-source /path/to/OpenRGB
tools/install-openrgb-plugin.sh --uninstall
```

The plugin is compiled against the headers of the installed OpenRGB version
(plugin API 5, OpenRGB 1.1); without `--openrgb-source` the script clones the
`release_<version>` tag from the official OpenRGB repository into a temporary
directory and removes it afterwards. It installs into
`~/.config/OpenRGB/plugins/`; restart OpenRGB to load it.
`tools/uninstall-user.sh --confirm` removes it too.

**OpenRGB and the app take turns.** Every write records who made it
(`lighting_source` in the status cache: `openrgb` or `soniccore`). While
OpenRGB has written in the last five seconds, the app's lighting card
says so and locks its color controls instead of letting a change vanish
under OpenRGB's next frame. **Assumir controle** runs
`soniccore lighting-hold on`: from then on the CLI refuses lighting writes
from OpenRGB (the plugin pauses and logs it once) and the app works
normally; **Devolver ao OpenRGB** runs `soniccore lighting-hold off`.
Changes made outside OpenRGB are mirrored back into it within about two
seconds, so its device page shows the colors the headset actually has.

All ten colors are written with one `soniccore set leds` command (logo 1-5,
then ring 1-5), the same lighting-profile reports as any other color change.
Each write sends a full profile to the headset, so the plugin sends at most
one write every 150 ms and always the newest state: fast OpenRGB effects are
reduced to that rate. The plugin sources are MIT licensed; because they are
compiled against OpenRGB (GPL-2.0-or-later) headers, the built plugin binary
is distributed under the GPL.

## Repository layout

- `docs/protocol/`: evidence ledger and protocol hypotheses;
- `docs/reverse-engineering.md`: capture workflow and safety gates;
- `docs/windows-vm-capture.md`: controlled QuantumENGINE/USBPcap workflow;
- `docs/plan-v0.1.md`: milestone definition;
- `tools/`: passive host/device inventory utilities;
- `openrgb-plugin/`: optional OpenRGB plugin that drives the headset through the CLI;
- `packaging/app/`: the desktop app — `main.qml`, the interface in `contents/`,
  Plasma's and Qt's controls in `controls/`, and the Qt launcher in `launcher/`;
- `packaging/flatpak/`: the app as a Flatpak, for Ubuntu 24.04-based systems;
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
JamBaLinux Sonic Core
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
equalizer for streams already destined for Game, with immutable factory presets
plus a user-owned library of named custom profiles; routing is automatic and
reversible, while Chat, microphone, and the system default route remain outside
the processing policy. Its current filters are a Linux approximation, not a
claim of QuantumENGINE parity. Real PipeWire/headset acceptance on 2026-09-19
covered browser continuity, measured gain changes, widget/profile/DSP reset,
Game-only isolation, default-route recovery, reversible routing, and a physical
dongle reconnect. Software Game/Chat mixing remain future work. Experimental
open spatial/binaural processing is implemented as a disabled-by-default,
host-side 7.1-to-stereo PipeWire chain feeding the existing EQ; its synthetic
HRIR hardware validation covered per-channel correlation, EQ effect, ten-minute
stability, safe fallback, and physical dongle reconnection. This work adds no
device-side audio functionality and does not change USB/HID safety boundaries.

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
