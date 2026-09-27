# Quantum 810 HID report descriptor

Status: **Observed**, not decoded semantically.

The Linux kernel exported a 553-byte report descriptor for HID interface 5.
Its SHA-256 is
`8b4f587e29256e8785ab98532208eaf9db35cf0732ca7e527541152c2c959691`.
A hexadecimal fixture is stored under `tests/fixtures/`.

## Structure

- Three HID collections.
- Standard usage pages: LEDs (`0x0008`), Button (`0x0009`), and Telephony
  (`0x000b`).
- Vendor-defined usage pages: `0xff13` and `0xff99`.
- Small input reports use IDs `0x02`, `0x03`, `0x06`, `0x07`, `0x08`,
  `0x09`, `0x10`, `0x2f`, and `0x9b`.
- Output report `0x06` has 61 payload bytes (62 bytes including Report ID).
- Output report `0x2f` has one payload byte (two bytes on wire).
- Numerous Feature Reports range from one to 63 payload bytes.
- Feature IDs `0x5b`, `0x71`, `0x9a`, and `0xf4` each occupy the maximum
  observed 63-byte payload (64 bytes including Report ID).

These sizes describe transport layout only. They do not establish the meaning
or safety of any command. No Output or Feature Report was sent or requested
during this observation.

Output Report `0x2f` declares six LED usages, including LED Usage `0x09`
(Mute). The headset also has a physical microphone LED that lights while muted.
This is a useful correlation, but control ownership remains unconfirmed and no
output has been attempted.

## hidraw interface reporting

The read-only `monitor --dry-run`, `status --dry-run`, and `inspect` commands
report the selected hidraw node's USB `bInterfaceNumber` from sysfs. The
repository's udev rule documents interface `05`; a different, missing, or
malformed value produces a warning. This is diagnostic evidence only: hidraw
selection still uses the existing HID identity criterion and does not require
interface `05`.

Before interface `05` can become a selection requirement, the owner must
validate that condition on the physical supported headset and record the
result in the evidence ledger. No VID/PID, allowlist, udev rule, HID command,
or report behavior is changed by this reporting step.
