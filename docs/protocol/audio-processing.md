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

### WirePlumber stream restoration and routing

When WirePlumber's `stream-restore` module restores application streams directly
to the virtual equalizer sink (because the application previously played through
it), the routing supervisor checks whether the stream's application identity
(`stream_key`) was previously proven and routed from the physical Quantum Game
sink (`proven_stream_keys`). If proven, the stream is immediately registered with
its original destination as the Game sink and remains processed (by EQ or promoted
to spatial audio) rather than being put into `unproven_streams` quarantine and
evacuated to raw physical output. If unproven, the quarantine and evacuation to
the safe default sink remain in effect. Furthermore, any stream manually moved
by the user to a different output (such as HDMI or analog) is never hijacked back.
When the spatial graph is stopped, all streams on the spatial capture sink are
safely evacuated to the equalizer sink before the graph process is terminated.


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

Identifiers are opaque strings generated by the CLI (`custom-<pid>-<sequence>`,
plus the reserved `custom-legacy-<n>` described under migration below) and never
change for the lifetime of a profile, so renaming is safe for scripts that
stored an ID. Names are user-facing: surrounding whitespace is trimmed,
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

The migrated custom profile does not get a freshly generated identifier: it
takes the reserved `custom-legacy-1`, derived only from the contents of the
file being migrated. The suffix moves past `1` only for a hand-edited schema 1
document that already carries a colliding custom identifier or name, so the
result never depends on the process, a counter or the clock. Migrating the same
schema 1 file twice therefore produces the same bytes, and repeated reads
always report the same `active_profile_id`, which is what lets the widget and
scripts poll a not-yet-migrated file without seeing the selection change under
them.

Migration never discards or alters gain values, and reading is pure: a
read-only command such as `soniccore equalizer status` migrates the document in
memory and writes nothing. The schema 2 form is written back atomically exactly
once, by the first command that already holds the configuration mutation lock,
inside that same serialized transaction; no separate lock is taken and no extra
rewrite happens afterwards, because the next read finds schema 2 and takes the
pure path. A file whose `schema` is neither 1 nor 2 is refused, because
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
stereo wet output. In parallel, its dry path makes an ITU-style 7.1-to-stereo
downmix: left is `FL + 0.707·FC + 0.707·SL + 0.707·RL`, right is
`FR + 0.707·FC + 0.707·SR + 0.707·RR`; LFE is intentionally omitted. Final
per-channel mixers start wet at `1.0` and dry at `0.0`, so the rendered output
is identical to the former binaural-only graph until a future live transition
changes those controls. Thus a stereo source using only FL/FR exits dry as
left=FL and right=FR. The PipeWire `channelmix` behavior for a stereo client
connected to the 7.1 sink remains an upmix assumption to verify on hardware.
That output disables session-manager autoconnection and is
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

## Diagnóstico de transições

Two user-visible symptoms are under investigation: (A) toggling spatial audio
pauses media that is playing, for example a YouTube video in a browser, and
(B) moving an equalizer band shows a "loading" delay instead of applying
immediately. This section records the hypotheses derived from static analysis
of the current code and the read-only tool that collects the evidence needed
to confirm or reject them. None of the hypotheses below is confirmed yet; they
are the candidates the measurements must discriminate between.

### Read-only diagnostic tool

`tools/diagnose-audio-transition.sh` only observes. It never moves streams,
changes the default sink, sets node parameters, creates links, starts or stops
services, or changes the SonicCore configuration.

- `tools/diagnose-audio-transition.sh spatial [--duration 20] [--interval 0.25] [--output FILE]`
  records, with a relative timestamp per line, every `pactl subscribe` event
  as it arrives plus snapshots, written only when they change, of: the stored
  `spatial.json` gate, the default sink, all sinks with their state, every sink
  input with its sink index, `Corked` flag, application and media name, the
  state of every PipeWire audio node from `pw-dump`, and the MPRIS
  `PlaybackStatus` of every player (via `busctl --user`, or `playerctl` when
  `busctl` is absent). The default log is
  `$XDG_RUNTIME_DIR/jambalinux-soniccore/diagnose-audio-transition-<time>.log`;
  an existing file is never overwritten.
- `tools/diagnose-audio-transition.sh timing [--runs 3] [--soniccore PATH]`
  reports min/median/max wall time of `soniccore equalizer status --format json`
  and `soniccore spatial status --format json`, and of their building blocks
  `pw-dump`, `pactl list sinks`, and `/bin/sh -lc true` (the login-shell
  wrapper the widget puts around every command).

Both modes require `pactl` and `pw-dump` and exit with a non-zero status and a
one-line message when either is missing. `python3` (node summary) and
`busctl`/`playerctl` (MPRIS) are optional; their absence is written to the log.

### A — spatial toggle pauses playback

The spatial switch only records intent in `spatial.json`
(`spatial::set_enabled`). Two independent 500 ms loops then act on it: the
spatial supervisor (`src/spatial_pipewire.rs`, `run`) starts or kills a child
`pipewire -c` process that owns the 7.1 spatial sink, and the equalizer route
supervisor (`src/pipewire.rs`, `run_route_iteration`) moves registered
streams between the stereo EQ sink and the spatial sink.

**H1 — the stream is moved between a 2-channel and an 8-channel sink.**
On enable, once the spatial output is linked to the EQ, `run_route_iteration`
selects the spatial sink as `processing_sink` and promotes every registered
stream from the EQ sink to it with `pactl move-sink-input`
(`registered_route_needs_promotion`). On disable, `restore_streams` moves them
back with the same command. Each move reconnects the client stream to a sink
with a different channel map (stereo ↔ 7.1), so pipewire-pulse renegotiates
the stream and the client receives a move/format change. Browsers that treat
an output-device change as a device loss may pause the media element or the
MPRIS player. Expected evidence: an `inputs` line whose `sink=` changes from
the EQ index to the spatial index (or back) while every node stays present,
followed within a few hundred milliseconds by `corked=yes` and/or an MPRIS
`Paused` line.

**H2 — streams are still attached when the spatial graph is killed.**
`stop_graph` (`src/spatial_pipewire.rs:331`) calls `restore_streams()` and
immediately kills the child PipeWire process. `restore_streams` snapshots the
sink inputs once, and `register_spatial_fallback_streams` releases the route
lock before the moves happen. During that window the equalizer route
supervisor still sees a live, connected spatial sink and a registered stream
on the EQ, so it can promote the stream back to the spatial sink; a stream
created after the snapshot is not moved at all. When the process dies, the
sink disappears under those streams, and the session manager moves them to a
fallback or the client sees its device vanish. Expected evidence: a
`pulse ... Event 'remove' on sink #<spatial>` while an `inputs` line still
shows that sink index, followed by the input moving to a sink other than the
EQ, disappearing, or corking.

**H3 — the default sink changes transiently.**
When a managed processing sink becomes the default, `run_route_iteration` and
`prepare` reset it with `pactl set-default-sink` on their next iteration. If
the session manager briefly selects the new spatial sink as default when it
appears, or selects another sink when it disappears, streams that follow the
default are moved by the session manager itself for up to one iteration
(≤ 500 ms), independently of H1. Expected evidence: `default` lines that
change and change back around the toggle.

The hypotheses are not exclusive; the log orders their effects in time, so the
first event that precedes the pause identifies the cause to fix first.

### B — equalizer band latency decomposition

A band change in the widget currently travels this path:

1. **Debounce.** `equalizerCommit` (`main.qml`, `interval: 180`) waits 180 ms
   after the last slider move or release before sending anything.
2. **Login shell.** Every command runs as `/bin/sh -lc "$HOME/.cargo/bin/soniccore …"`,
   so each call sources the user's login profile before `soniccore` starts.
3. **CLI apply.** `soniccore equalizer set` calls `commit_equalizer_profile`,
   whose `pipewire::apply_profile` runs `pw-dump` once to find the node and
   read current gains, four `pw-cli set-param` ramp steps separated by three
   10 ms sleeps, and a second full `pw-dump` to verify node identity and the
   applied gains, and only then persists the profile. The 2026-09-19
   acceptance measured about 0.10 s for this call end to end.
4. **Full status after every mutation.** When the action finishes, the widget
   always calls `refreshEqualizer()`, i.e. another login shell plus
   `soniccore equalizer status --format json`, whose `pipewire::status` runs
   `systemctl --user is-active`, `pw-dump`, three `pactl` queries (sinks, sink
   inputs, default sink), and reads the routing state file twice.
5. **Blocked UI.** While `equalizerBusy` or `equalizerUpdating` is true,
   `startEqualizerAction` returns `false`. A band moved during that time is not
   queued: its debounced commit is dropped, and the following status refresh
   rewrites the handles from the applied gains, so the slider can snap back to
   its previous value.

The perceived delay is therefore roughly *180 ms + 2 × login shell + CLI apply
+ full status*, with any overlapping input lost. The `timing` mode measures
steps 2 and 4 and their `pw-dump`/`pactl` components directly; step 3 can be
timed manually as shown below. The live DSP update itself (the ramp) is only a
nominal 30 ms, and the node is not recreated, so rebuilding the graph is not
part of the delay.

### Manual reproduction procedure

1. Build and install the current tree, confirm `soniccore equalizer status`
   reports `active` and that spatial is disabled and ready
   (`soniccore spatial status`).
2. Start media in the browser (for example a YouTube video) and confirm it
   plays through the Game output.
3. In a terminal run `tools/diagnose-audio-transition.sh spatial --duration 30`.
4. Within the recording window, enable spatial audio in the widget (or with
   `soniccore spatial enable`), wait about ten seconds, then disable it (or
   `soniccore spatial disable`). Note the wall-clock moments of each click and
   whether playback paused.
5. Inspect the log: find the first `mpris ... Paused` or `corked=yes` line and
   read the `inputs`, `default`, `sinks`, `nodes`, and `pulse` lines just
   before it to decide between H1, H2, and H3.
6. For B, run `tools/diagnose-audio-transition.sh timing --runs 5`. To time
   the write path, note the current gain of one band, run
   `time soniccore equalizer set 500 <new>` and then restore it with
   `time soniccore equalizer set 500 <previous>`.
7. Attach the log and the timing table to the fix phase that addresses the
   confirmed hypothesis. The log contains application and media names; review
   it before sharing it publicly.
