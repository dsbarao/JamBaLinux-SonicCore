# JamBaLinux Sonic Core v0.4 plan — sound modes

Goal: give the app the sound options people expect from headset software on
Windows (virtual surround, listening modes, dialogue and bass control), built
only from what PipeWire already ships, with no "experimental" wording in the
interface.

## What exists today

- A 7.1 virtual surround sink ("JamBaLinux espacial"): eight inputs, sixteen
  HRIR convolvers in the HeSuVi layout, mixed to stereo and sent into the
  equalizer. This is the same technique as commercial headphone surround.
- A wet/dry mix that is changed live (no graph rebuild): `wetDryL/R`.
- The equalizer, now with a natural-sound bypass.
- PipeWire 1.6 on this machine ships filter-graph plugins `builtin`
  (convolver, biquads, mixer, delay), `sofa`, `ebur128`, `ladspa` and `lv2`.

## 1. Listening modes (first delivery)

One selector in the spatial card: **Natural · Jogo · Cinema · Música**. Each mode
is a set of values for controls that already exist or are cheap to add, all
changed live:

| Mode    | 3D amount | Centre (dialogue) | LFE (bass) | Extra                          |
|---------|-----------|-------------------|------------|--------------------------------|
| Natural | 0 (dry)   | 0 dB              | 0 dB       | —                              |
| Jogo    | 100 %     | 0 dB              | −3 dB      | surround +2 dB (footsteps)     |
| Cinema  | 100 %     | +3 dB             | +3 dB      | —                              |
| Música  | 40 %      | 0 dB              | 0 dB       | surround −3 dB                 |

- [x] Add `centre` and `lfe` gain controls to the existing mixers (the graph
  already has the slots: `dryL/R` Gain 2–4 and the FC/LFE convolvers).
- [ ] Add a small post-filter stage in the spatial graph (two `bq_peaking`
  per side) for the mode's tonal touch, separate from the user equalizer.
  (Later: it changes the graph, so it needs one announced rebuild.)
- [x] `soniccore spatial sound <game|cinema|music>` and
  `soniccore spatial sound set <intensity|dialogue|bass|surround> <value>`,
  stored in `spatial.json` and ramped live; Natural is the 3D switched off.
- [x] Fine sliders under the selector: **Intensidade 3D**, **Diálogos**,
  **Graves**, **Surround**. Moving one turns the mode into "Personalizado".

## 2. Sound signatures (HRIR choice)

- [ ] Let the user keep several HRIR files in `spatial/hrtf/` and pick one
  ("Assinatura 3D"). Changing it rebuilds the spatial graph once (a short,
  announced gap; never while the user drags a slider).
- [ ] Neutral, generic names only. No vendor trademarks in the UI, and the
  app never downloads or redistributes proprietary captures.
- [ ] Optional: a SOFA-based mode using PipeWire's `sofa` plugin, with a
  freely licensed dataset the user installs (licence shown next to it).

## 3. Volume balance (night mode)

- [ ] **Volume equilibrado**: loudness normalisation with the `ebur128`
  plugin so explosions and whispers stay closer, for late-night play.
- [ ] Off by default; one switch in the equalizer card.

## 4. Getting games into the 3D sink without menus

- [ ] A list of the apps playing now with a **3D** toggle per app, moving that
  stream to the spatial sink (and back) with PipeWire metadata, so the user
  does not have to pick the output inside each game.
- [ ] Remember the choice per application name.

## Interface rules

- No "experimental", "dataset", "bypass" or "saudável" in the UI; states are
  Ligado / Desligado / Ligando… / Indisponível neste computador.
- Every option is visible at once in its card; unavailable parts fade, chosen
  modes animate the controls they change.

## Safety

- Only PipeWire graphs change; no USB/HID behaviour is touched.
- Live controls ramp like the equalizer does, so a mode change never clicks.
- A failed update rolls back to the last confirmed values, as the equalizer
  already does.
