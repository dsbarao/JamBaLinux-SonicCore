# Protocol research

Early HID protocol mappings are now documented through controlled passive
observations and QuantumENGINE USBPcap comparisons. JamBaLinux SonicCore
implements only the confirmed controls in its strict allowlist. Other observed
vendor writes remain research documentation and must not be replayed blindly.
Experiment notes preserve the scope and observations at the time of each capture.

Current consolidated mappings:

- [`lighting.md`](lighting.md): lighting state, zones, segments, RGB, and known
  effect identifiers.
- [`microphone.md`](microphone.md): device versus host-side microphone controls.
- [`ambient-control.md`](ambient-control.md): ANC and TalkThru command/confirmation
  mapping.
- [`audio-processing.md`](audio-processing.md): host-side output DSP controls.

Use `evidence-ledger.md` to index experiments and sanitized artifacts. Keep
raw captures outside Git in `captures/`. Protocol documents must distinguish:

- **Observed:** bytes or descriptors directly measured;
- **Hypothesis:** a proposed meaning not yet validated;
- **Confirmed:** reproduced with controlled changes and negative controls.
