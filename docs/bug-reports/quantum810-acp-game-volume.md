# Draft: JBL Quantum 810 Wireless — Game volume drives the Chat hardware control

Status: **draft, not submitted.** Target: PipeWire `alsa-card-profile`
(spa/plugins/alsa/mixer). Fill in the bracketed fields before filing.

## Environment

- PipeWire: [version, e.g. 1.6.9]
- WirePlumber: [version]
- Distribution / kernel: [fill in]
- Device: JBL Quantum 810 Wireless USB dongle, `0ecb:2069`
- udev rule: `90-pipewire-alsa.rules` sets `ACP_PROFILE_SET=usb-gaming-headset-gamefirst.conf`

## Mixer controls exposed by the dongle

```
Simple mixer control 'PCM',0    -> playback PCM 0 (Game, hw:X,0)
Simple mixer control 'PCM',1    -> playback PCM 1 (Chat, hw:X,1)
Simple mixer control 'Mic',0    -> capture
```

## Steps to reproduce

1. Plug the dongle; the card uses `usb-gaming-headset-gamefirst.conf`.
2. Set the Game sink volume to 64 %: `wpctl set-volume <game-sink> 0.64`.
3. Read the controls: `amixer -c X sget PCM,0` and `amixer -c X sget PCM,1`.

## Expected

The Game sink volume drives `PCM,0`; the Chat sinks drive `PCM,1`.

## Observed

The Game sink drives `PCM,1` (about -11 dB at 64 %, cubic volume), and `PCM,0`
is never driven. It stays at whatever value was last stored (-23 dB on the
reporting host), so the Game output is far quieter than intended and users
compensate digitally, which clips. Setting `PCM,0` to 0 dB by hand restores
the expected level and fidelity.

## Cause

In `usb-gaming-headset-gamefirst.conf`, `[Mapping stereo-game-output]`
(`hw:%f,0,0`) uses `paths-output = usb-gaming-headset-output-stereo`, whose only
element is `[Element PCM,1]`. `[Mapping mono-chat-output]` (`hw:%f,1,0`) uses
`usb-gaming-headset-output-mono`, which drives `[Element PCM]`. For this device
the indices are swapped: the Game PCM needs `[Element PCM]` (as in
`steelseries-arctis-output-game-common.conf`) and both Chat mappings need
`[Element PCM,1]`.

## Suggested fix

Give the gamefirst profile-set device-specific paths: Game → `[Element PCM]`,
Chat stereo/mono → `[Element PCM,1]`. The JamBaLinux user overlay
(`packaging/alsa-card-profile/`) is a working example. The same profile-set
is used by the JBL Quantum One (`0ecb:203a`), which was not tested here.
