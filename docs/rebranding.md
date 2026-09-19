# JamBaLinux SonicCore: technical identity

The public software name is **JamBaLinux SonicCore** and its technical
identifier is `jambalinux-soniccore`. The command-line client is `soniccore`;
the daemon executable is `soniccore-daemon`; and the user unit is
`jambalinux-soniccore.service`.

This project was made public before its first release or installation. There
are no deployed users, state files, widgets, desktop entries, D-Bus clients, or
scripts requiring a compatibility transition. The project therefore uses the
new identifiers directly, without legacy aliases or migration code.

## Technical identifiers

| Interface | Identifier |
|---|---|
| Cargo package | `jambalinux-soniccore` |
| CLI | `soniccore` |
| Daemon | `soniccore-daemon` |
| systemd user unit | `jambalinux-soniccore.service` |
| Plasma widget | `org.jambalinux.soniccore` |
| D-Bus name and interface | `org.jambalinux.soniccore.State` |
| D-Bus object path | `/org/jambalinux/soniccore/State` |
| Runtime cache | `$XDG_RUNTIME_DIR/jambalinux-soniccore-state.json` |
| Desktop entry | `jambalinux-soniccore.desktop` |
| udev rule | `70-jambalinux-soniccore.rules` |
| Project website | `https://github.com/dsbarao/JamBaLinux-SonicCore` |

The rebrand does not change HID reports, USB matching, VID/PID values,
allowlists, or protocol behavior. **JBL Quantum 810 Wireless** identifies the
supported hardware, while `QuantumENGINE`, USB strings, captures, fixtures, and
reverse-engineering evidence identify vendor software or observed behavior and
are intentionally preserved.

## License attribution

The MIT license identifies Daniel Barão as the copyright holder. The license
text is otherwise unchanged.
