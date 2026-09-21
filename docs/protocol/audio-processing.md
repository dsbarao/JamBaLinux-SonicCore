# Output audio processing

## Equalizer

QuantumENGINE exposes a 10-band output equalizer at 31, 62, 125, 250, and
500 Hz and 1, 2, 4, 8, and 16 kHz, each ranging from -12 dB to +12 dB.

Changing from Bass Boost to Flat and back generated no USB traffic (EVT-034).
The equalizer is host-side DSP, not a dongle/headset setting. A Linux
implementation should use PipeWire filters, EasyEffects-compatible processing,
or another host DSP layer rather than vendor USB writes.

## JamBaLinux profile and persistent service

JamBaLinux stores the active 10-band setting, the current selection, and a
library of user-named custom profiles in one versioned file under
`$XDG_CONFIG_HOME/jambalinux-soniccore/equalizer.json` (or
`~/.config/jambalinux-soniccore/equalizer.json`). The `soniccore equalizer`
command and Plasma widget let users edit the documented bands within the
documented -12 dB to +12 dB range. This file is intentionally independent
of HID state and does not open a headset device or send any USB report.

The user installation includes the persistent
`jambalinux-soniccore-equalizer.service`. It starts automatically with the user
session, waits when the headset is unavailable, and retries discovery rather
than requiring an activation button. It owns one PipeWire filter-chain and a
virtual **JamBaLinux Game Equalizer** sink for the lifetime of the service.

`equalizer set` and `equalizer reset` issue updates to the existing node through
PipeWire's `Props` parameter. Each `Props` update carries all ten controls
together. A change schedules four updates separated by three 10 ms intervals:
a nominal 30 ms ramp that excludes `pw-cli` execution and scheduling overhead.
The command then reads the controls from the same node and object serial before
saving the profile. On the 2026-09-19 live-graph acceptance run, complete
commands took about 100 ms and the audio effect was measured independently.
That end-to-end time is not a sample-accurate measurement of the ramp itself;
the ramp remains a nominal 30 ms schedule. The update path does not invoke
`systemctl restart` or deliberately recreate the filter node. If the live
update or profile save fails, the command reports the failure and attempts to
restore the preceding control values.

The output is pinned only to the unique Quantum Game PCM: USB playback PCM 0
with `alsa.components = USB0ecb:2069`. Discovery refuses an absent or ambiguous
target instead of guessing. The routing loop considers only playback streams
whose current destination is that proven Game sink. Before moving one to the
virtual sink, it persists the stream's `pactl` index and restore identifier
together with the original sink name and `pactl` index. On service shutdown,
matching live streams are moved back by sink name, with the recorded sink index
as a fallback.

The virtual sink has zero session and driver priority and is never deliberately
made the default. The service records the preceding non-virtual default and
restores it if the virtual sink becomes default. It never selects Chat streams
or capture nodes, so Chat playback and microphones remain outside the chain.
Health checks detect links from the equalizer output to the headset's Chat or
capture nodes and report the chain as unhealthy; they do not remove those
links.

`soniccore equalizer status` and the widget expose actionable errors for an
inactive service, unavailable Game output, missing or disconnected filter,
unsafe default or links, unobservable DSP controls, and a mismatch between the
live controls and saved profile. They do not present the equalizer as active
when these checks fail.

The filter shape is an initial Linux approximation: 31 Hz uses a low shelf,
62 Hz through 8 kHz use peaking biquads, 16 kHz uses a high shelf, and every
band uses Q=1.0. USB captures confirm only the centers and gain range, not
QuantumENGINE's Q or filter topology, so this is not claimed to be acoustically
identical to the Windows implementation. Large positive gains can clip; there
is currently no automatic preamp.

### Captured predefined profiles

The maintainer supplied screenshots from the original software for 17 output
equalizer profiles under `ExemploEqualizadoresPreDefinidos/`. All screenshots
use the same ten documented center frequencies and show integer-dB handle
positions. JamBaLinux transcribes those positions as complete, atomic profiles:

| Profile | 31 | 62 | 125 | 250 | 500 | 1k | 2k | 4k | 8k | 16k |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Flat | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Bass Boost | 6 | 6 | 4 | 2 | 0 | 0 | 0 | 2 | 1 | 0 |
| Cinematic | 4 | 3 | 2 | 0 | -1 | 1 | 2 | -1 | -2 | -2 |
| FPS | -5 | -3 | -1 | 0 | -1 | 1 | 4 | 2 | -1 | 0 |
| MOBA | 3 | 4 | 2 | 1 | -1 | -1 | 1 | 2 | -3 | -3 |
| RPG | 3 | 3 | 3 | 0 | -2 | -2 | -2 | 1 | 2 | -2 |
| Apex Legends | -6 | 3 | 2 | -3 | 2 | 1 | 2 | 4 | 1 | -4 |
| CS2 | -6 | -4 | -3 | -2 | 2 | 3 | 4 | 4 | 3 | 0 |
| Dota 2 | 4 | 3 | 1 | -4 | -1 | 3 | 4 | 1 | 3 | -4 |
| Fortnite | -4 | 3 | 2 | -3 | -4 | 3 | 3 | 4 | 2 | -4 |
| GTA 5 | -1 | 1 | 2 | -3 | 3 | 4 | 3 | 3 | 2 | -2 |
| LoL | 2 | 3 | 1 | -3 | -4 | 1 | 2 | 4 | 1 | -3 |
| PUBG | -1 | -2 | -1 | -1 | 2 | 3 | 0 | 5 | 4 | -2 |
| WoW | 3 | 2 | 2 | -1 | -2 | -1 | 1 | 3 | -2 | -2 |
| Escape from Tarkov | -4 | 3 | -3 | 6 | 3 | 1 | 3 | 6 | 2 | 1 |
| LN3 Immersion | 4 | 5 | 4 | 1 | -3 | -2 | 1 | 3 | 1 | -2 |
| LN3 Thrill | 1 | 0 | -1 | -5 | 2 | 1 | 3 | 4 | 2 | 1 |

Selecting one profile updates all ten bands in one normal live-DSP transaction.
These gain values reproduce the visible control points, but acoustic parity is
not claimed because the original software's Q values and filter topology remain
unknown.

The seventeen profiles above are factory presets and are immutable. They live
in the program's own `PRESETS` table, are never written into the configuration
file as editable entries, and cannot be updated, renamed, or deleted. Their
identifiers (`flat`, `bass-boost`, `fps`, `ln3-thrill`, …) are reserved: the
library rejects any custom profile that claims one of them.

### Custom profile library

Schema 2 of `equalizer.json` holds the library alongside the active bands:

```json
{
  "schema": 2,
  "bands": [ { "frequency_hz": 31, "gain_db": 4.0 }, "… ten bands …" ],
  "active_profile_id": "custom-4711-0",
  "custom_profiles": [
    {
      "id": "custom-4711-0",
      "name": "Noite",
      "bands": [ { "frequency_hz": 31, "gain_db": 4.0 }, "… ten bands …" ]
    }
  ]
}
```

`bands` remains the single source of truth for the live DSP; the library is
additional user data. `active_profile_id` names either a factory preset ID or a
custom profile ID, and is `null` when the current bands are unsaved manual
settings. Loading validates the whole file: the active bands and every stored
profile must carry exactly the ten documented frequencies with finite gains
inside -12 dB to +12 dB, which are normalized to one decimal place on load, and
a non-`null` `active_profile_id` must exist and match `bands` exactly. A file
that violates any of these rules is rejected with an error rather than silently
repaired.

Identifiers are opaque strings generated by the CLI (`custom-<pid>-<sequence>`)
and never change for the lifetime of a profile, so renaming is safe for scripts
that stored an ID. Names are user-facing: surrounding whitespace is trimmed,
control characters are rejected, an empty name is rejected, the length limit is
64 characters, and uniqueness is enforced case-insensitively across custom
profiles. A name that collides with an existing one is refused instead of
producing a second entry with the same label.

Mutations behave as follows:

- **Create** copies the current ten bands into a new named profile and selects
  it.
- **Apply** replaces the active bands with a stored definition and records the
  selection. Factory IDs are refused here; they keep the dedicated `preset`
  path.
- **Update** overwrites a custom profile's ten bands with the currently active
  bands, keeping its ID and name.
- **Rename** changes only the label. It never touches the live DSP, because no
  audible value changes.
- **Delete** removes a custom profile. Deleting the profile that is currently
  selected applies the immutable **Flat** preset in the same transaction, so the
  session never ends with a selection pointing at a profile that no longer
  exists; deleting any other profile only rewrites the file and leaves the
  audible setting alone.

Editing a single band while a custom profile is selected writes the new value
into that profile, which keeps the selector and the audible result consistent.
Editing a band while a factory preset is selected leaves the preset untouched
and clears the selection to unsaved manual settings; saving that state as a
profile is an explicit *create*.

Every mutation that touches audio runs inside the existing transaction: the
configuration mutation lock is held, the live PipeWire `Props` update is applied
and read back, and only then is the file written atomically. A failure restores
the preceding control values and leaves the stored library unchanged.

### Migration from the previous single-profile file

Schema 1 files stored only `schema` and `bands`. They are migrated on load,
without user action and without a separate backup step:

- if the stored bands exactly match a factory preset, that preset becomes the
  selection and no custom profile is created;
- otherwise the bands are preserved as a custom profile named `Personalizado`,
  which becomes the selection, so a hand-tuned setting survives as a named,
  re-selectable entry instead of being lost or silently renamed to a preset.

Migration never discards or alters gain values. Reading a schema 1 file does not
rewrite it; the migrated schema 2 form is persisted the next time a command
saves successfully. A file whose `schema` is neither 1 nor 2 is refused, because
guessing the meaning of an unknown future layout could apply wrong gains.

### JSON consumed by the widget and scripts

`soniccore equalizer status --format json` is the interface the Plasma widget
polls. Its object carries `schema`, the ten active `bands`, `active_profile_id`,
the `custom_profiles` array described above, `preset` (the identifier of the
factory preset whose gains match the active bands, or `null`), and the
`pipewire` health object. `preset` is derived from the gains alone, so it is
also filled in when the selection is a custom profile that happens to match a
preset. `soniccore equalizer profile list --format json` emits the library
subset only: `schema`, `active_profile_id`, and `custom_profiles`.

The widget treats this output as a read-only view: the factory list is its own
constant in CLI order, custom entries are rebuilt from the last status response,
and the CLI stays the only writer of the library. Scripts should do the same.
These fields are the ones currently implemented; no stability guarantee is made
beyond them, and consumers should tolerate additional fields and check `schema`
rather than assume a fixed object shape.

### Boundary

Profile management changes host-side DSP state only. None of the profile
commands opens a hidraw node, issues a HID request, or sends a USB report, and
none of them alters the routing policy: the processed destination remains only
the proven Quantum **Game** output, while Chat playback, capture sources, and
the microphone stay outside the chain exactly as described above.

## Real-system acceptance (2026-09-19)

The recovery implementation was exercised on the maintainer's JBL Quantum 810
Wireless with PipeWire 1.6.8. These observations validate this host and graph;
they are not a claim of acoustic parity with QuantumENGINE.

| Criterion | Recorded evidence |
|---|---|
| Automatic persistent chain | The user service remained `active/running` with PID `1485189`, `NRestarts=0`, and start timestamp `92705227768`. The filter stayed at node `90`, object serial `8925`, with output serial `8926`. There was no manual activation control. |
| Ten live controls and measured effect | All ten expected `Props` controls were readable from the same node. At 500 Hz, a +6 dB setting measured -35.58 dBFS versus -42.27 dBFS at 0 dB, a +6.69 dB difference. At the 16 kHz high-shelf corner, +12 dB measured -45.56 dBFS versus -51.61 dBFS at 0 dB, a +6.05 dB difference consistent with the shelf's corner response. |
| Playback continuity | A local 60-second Chrome video with audio remained `paused=false` and `ended=false`, with time advancing and looping while 500 Hz and 16 kHz values changed. Its proven Game stream was routed through the equalizer during the test. The service PID, restart count, node ID, and object serial did not change. |
| Nominal ramp and no restart | Four complete ten-control updates were separated by three 10 ms waits, giving the required nominal 30 ms schedule. Observed end-to-end CLI calls were approximately 0.100-0.101 s; this includes process and verification overhead and is not reported as the ramp duration. No band change restarted the service or recreated the node. |
| One reset action | Clicking the installed widget's **Zerar bandas** control made all ten visible values zero, saved all ten profile values as zero, and read back all ten live DSP gains as zero. PID/node identity stayed unchanged. The maintainer's pre-test 16 kHz +12 dB value was then restored through the widget. |
| Game-only processing | The chain linked only to the confirmed USB playback PCM 0 Game sink. No equalizer links reached Chat playback or the microphone. The saved and applied profiles stayed synchronized and `chat_isolated=true`. |
| Safe and reversible routing | Forcing the virtual sink as default was corrected back to the physical Game sink within the next observed routing iteration. A temporary stream that began on proven Game was moved to the equalizer only after its Game destination had been persisted. Moving that stream manually to physical analog sink serial `52` was respected, and its routing record was removed rather than hijacked back. |
| Physical reconnect | Disconnecting and reconnecting the dongle changed the physical Game object serial from `1820` to `16560`. The service PID, restart count, EQ node `90/8925`, and profile were preserved; the output links and persisted target were rebuilt for Game `16560`. A post-reconnect playback stream was observed as application -> EQ `8925` -> output `8926` -> Game `16560`, with no Chat or microphone link. |
| Health/error reporting | Final JSON status reported `active`, `service_active`, `target_connected`, `default_safe`, `chat_isolated`, `routing_healthy`, and `profile_synced` as true, with `error=null`. The only reconnect journal warning recorded the expected interval in which Game was physically absent. |

The dongle reconnect itself occurred with no application stream live. It proves
service, DSP-node, profile, target, and link recovery; the complete application
path was exercised immediately after reconnection. The browser continuity and
live-gain criteria were exercised separately while its stream was active.

## Spatial audio

Disabling and re-enabling spatial audio with DTS Headphone:X v2.0 selected
generated no USB traffic (EVT-035). The enable switch controls a host-side
spatial-processing chain; the headset receives the resulting audio stream and
does not store this state.

Selecting Quantum Spatial from DTS, changing room size from large to medium to
small, and changing head diameter from 25 to 20 to 15 cm also generated no USB
traffic (EVT-036). Mode selection and personalization geometry are therefore
host-side DSP parameters as well.

### Open spatial/binaural processing

JamBaLinux implements an experimental open, vendor-neutral spatial/binaural
processor — never the vendor's DTS or Quantum Spatial names or files, which
stay documented above solely as evidence about vendor software behavior.
`src/spatial.rs` stores a disabled-by-default gate and mode (`off` or
`binaural-stereo`) under
`$XDG_CONFIG_HOME/jambalinux-soniccore/spatial.json`. Its read-only capability
preflight validates the PipeWire executable and filter-chain module plus a
user-supplied 14-channel HRIR dataset under `spatial/hrtf/` in the
configuration directory. Technical validation does not grant legal clearance
for a third-party HRIR dataset.

When explicitly enabled, the persistent spatial supervisor creates one 7.1
virtual sink and convolves its eight inputs through sixteen HRIR paths into a
stereo output. That output disables session-manager autoconnection and is
linked explicitly, channel by channel, only to the existing equalizer input;
the complete authorized path is `application -> Spatial -> EQ -> Quantum
Game`. The spatial sink cannot become the default, and health fails closed if
the target, stereo links, Chat isolation, capture isolation, formats, or
dataset cannot be proven. Before removing a live graph, the supervisor
registers and transfers its streams to the exact EQ sink so playback can
continue without bypassing the equalizer.

The hardware acceptance on 2026-09-20 used a locally generated synthetic HRIR
fixture. It correlated all eight input channels digitally, measured the EQ
inside the chain, ran for 605 seconds with PipeWire `ERR=0`, exercised live
fallback, and recovered automatically after physical dongle reconnection.
Full measurements and the remaining legal boundary for a distributable HRIR
are recorded in
[`validation_reports/spatial-hardware-validation.md`](validation_reports/spatial-hardware-validation.md).
