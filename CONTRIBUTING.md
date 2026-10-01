# Contributing

JamBaLinux SonicCore accepts reproducible, safety-scoped Linux headset
research. The first supported device is the JBL Quantum 810 Wireless. Keep
software branding separate from actual hardware and protocol names; see
[rebranding](docs/rebranding.md).

For collaboration or bug reports, open a GitHub issue. For responsible
disclosure of a safety concern, contact [jambalinux@gmail.com](mailto:jambalinux@gmail.com).

Before submitting a change:

```bash
cargo fmt -- --check
cargo test
cargo clippy --all-targets -- -D warnings
udevadm verify packaging/udev/70-jambalinux-soniccore.rules
```

Protocol claims must link to an experiment in `docs/protocol/experiments/` and
distinguish observed facts, hypotheses, and confirmed mappings. Do not commit
raw captures, serial numbers, vendor binaries, firmware, or unrelated USB
traffic. Active output/feature reports require a separately reviewed safety
plan before extending the existing confirmed allowlist.
