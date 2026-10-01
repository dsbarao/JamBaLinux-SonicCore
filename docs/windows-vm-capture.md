# Windows VM capture plan

Target VM: `win11` under system libvirt/KVM.

## Observed baseline

- Windows 11, Q35, UEFI/Secure Boot, TPM 2.0
- 6 vCPUs, 16 GiB RAM
- raw VirtIO disk: `/home/daniel/DiscoVM/win11.img`
- qemu-xhci controller with two SPICE USB redirection slots
- no JBL USB host device currently attached
- no existing libvirt snapshots
- dongle remains owned by the Linux host as `0ecb:2069`
- XFS `/home` supports copy-on-write reflinks

## Safety sequence

1. Shut Windows down normally and confirm libvirt state `shut off`.
2. Create a read-only baseline reflink beside the VM image:

   ```bash
   cp --reflink=always --sparse=always \
     /home/daniel/DiscoVM/win11.img \
     /home/daniel/DiscoVM/win11-jambalinux-soniccore-baseline.img
   chmod a-w /home/daniel/DiscoVM/win11-jambalinux-soniccore-baseline.img
   ```

3. Record SHA-256 only if time permits; hashing 150 GiB is optional and slow.
4. Start the VM and install Wireshark/USBPcap and JBL QuantumENGINE from their
   official sources.
5. Disable or decline firmware update flows. Do not open firmware tools.
6. Pass only USB `0ecb:2069` to the VM through SPICE USB redirection.
7. Confirm the device disappears from Linux and appears in Windows before
   opening QuantumENGINE.
8. Capture a baseline application startup with no setting changes.
9. Change exactly one setting per capture and record an experiment ID.
10. Disconnect the USB device from the VM before restoring or replacing disks.

## Capture order

1. QuantumENGINE startup/status refresh
2. Unknown passive state Report `0x07`
3. Battery/status refresh
4. Lighting toggle
5. Sidetone
6. Equalizer preset change
7. Spatial audio controls

Firmware update traffic is explicitly out of scope.

## Raw capture handling

Store original `.pcapng` files under the repository's ignored `captures/`
directory or outside the repository. Record experiment metadata and SHA-256 in
the evidence ledger; commit only minimal sanitized excerpts when appropriate.
