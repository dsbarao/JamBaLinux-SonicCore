# Reverse-engineering workflow

## Phase 0 — passive Linux inventory (current)

1. Identify the dongle by VID/PID and stable sysfs attributes.
2. Map configurations, interfaces, alternate settings, endpoint directions,
   transfer types, packet sizes, and bound kernel drivers.
3. Correlate USB Audio interfaces with ALSA and PipeWire nodes.
4. Record observations without opening device nodes or sending traffic.

### Passive input monitoring

The `monitor` command dynamically resolves the Quantum 810 hidraw node through
its HID ID, verifies that the target is a character device, and opens it with
read-only access. It prints spontaneous Input Reports only. Start with
`monitor --dry-run`; no udev rule or elevated permission should be added until
the resolved node and existing access policy have been reviewed.

The packaged udev rule matches vendor `0ecb`, product `2069`, and USB interface
`05`, then applies `TAG+="uaccess"`. This delegates access to the active local
seat through systemd-logind rather than making the device world-writable. After
installation/reload, physically reconnect the dongle so the add event runs the
complete rule chain. To undo it, remove only
`/etc/udev/rules.d/70-jambalinux-soniccore.rules`, reload rules, and reconnect.

## Phase 1 — controlled vendor observation (later)

Use a disposable Windows VM with the official QuantumENGINE software,
USBPcap, Wireshark, and exclusive dongle passthrough. Establish a baseline,
then change exactly one UI setting per capture. Preserve originals locally and
commit only sanitized excerpts plus hashes and experiment notes.

Suggested experiment order: read-only application startup, battery/status
refresh, lighting toggle, sidetone, microphone mute behavior, equalizer, and
spatial-audio controls. Do not begin with firmware update flows.

## Phase 2 — decode and replay gate

Before replaying anything, identify direction, transport, report/control
request metadata, stable fields, counters/checksums, and device/firmware scope.
Replay requires a reviewed experiment note with the exact known-good bytes,
expected response, timeout, repetition limit, and recovery procedure.

## Evidence standard

Separate facts, observations, hypotheses, and conclusions. Every proposed
field in the protocol map must cite a capture hash and experiment ID. Two or
more controlled comparisons are required before naming a field.
