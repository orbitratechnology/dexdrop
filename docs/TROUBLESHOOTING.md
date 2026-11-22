# Troubleshooting

## The other device does not appear in Nearby

DexDrop finds devices with mDNS/DNS-SD (the same mechanism as AirPlay and
network printers) on the local network. Check, in this order:

1. **Both devices run DexDrop and are visible.** Settings → *Visible to nearby
   devices* must be on for the device you want to send *to*. Sending works
   either way.
2. **Same network.** Both on the same Wi-Fi or wired network. A phone on
   mobile data, or a laptop on a different Wi-Fi band that the router
   isolates, is not on the same network.
3. **Guest networks and "client isolation".** Many guest Wi-Fi networks,
   hotels, offices and some mesh routers block devices from seeing each
   other. Discovery cannot work there; the devices cannot reach each other
   directly either. Use a network without isolation (or a phone hotspot).
4. **VPNs.** A VPN can route local traffic away from the network. Pause it
   or allow local network access in its settings.
5. **Firewalls.** DexDrop needs UDP port 5353 (mDNS) and one TCP port it
   chooses when it starts (shown by `dexdrop receive`, or set a fixed one
   with `DEXDROP_PORT`). Windows asks the first time: allow *Private
   networks*. On Linux, open them in `ufw`/`firewalld` if enabled.
6. **Permissions.**
   - *iOS:* Settings → Privacy & Security → Local Network → DexDrop on.
   - *macOS:* System Settings → Privacy & Security → Local Network →
     DexDrop on.
   - *Android:* nearby discovery is not available in this build yet (see
     below); use a code or an invitation.

If discovery still fails, **pair with a code**: on the receiving device
choose *Receive with a code*; on the sending device choose *Send with a code*
and type it. Or copy the invitation, or read its QR code from a screenshot
with *Read QR from picture*. Invitations contain the receiver's addresses,
so they work even where discovery is blocked, as long as the devices can
reach each other directly.

Settings → *Nearby discovery is …* shows whether discovery is running, and
*Refresh* on the Nearby screen restarts it after a network change.

## "The device could not be reached"

The device was found but a direct connection failed: usually client
isolation, a firewall blocking the TCP port, or one device switched
networks. DexDrop retries on its own a few times; after that, press
*Retry* once both are on the same network.

## A transfer stopped

- **Interrupted, retrying:** the connection dropped (Wi-Fi blip, sleep).
  Nothing to do; it continues from the last verified chunk.
- **Interrupted, not retrying:** after several failed attempts it waits for
  you. Press *Retry*. Verified data is kept, so nothing is sent twice.
- **"This device ran out of storage space":** free space on the receiving
  device, then ask the sender to press *Resume*. The verified part is kept.
- **Paused by the other device:** the other side paused; it resumes from
  there.
- **Closed the app mid-transfer:** reopen it; the transfer is listed as
  interrupted and *Retry* (on the sender) continues it.

## A file failed

Each file is verified with BLAKE3 before it is saved. Messages you may see:

- *The file changed or was removed on the sending device* – it was edited,
  moved or deleted while being sent. Send it again.
- *The received data did not pass verification* – it was requested again
  once and still did not match. Check the sending device's storage.
- *The file could not be saved on the receiving device* – permissions or a
  name that clashes with a folder.

A file that fails is never saved, not even partly.

## Where are my files?

- *Desktops:* `Downloads/DexDrop` unless you chose another folder in
  Settings. *Show in folder* opens it.
- *Phones:* in DexDrop's own storage; use *Share or save* on the finished
  transfer to put them in Files, Photos or another app.
- Names that already exist are never overwritten unless you chose
  *Replace* (and confirmed). By default you get `name (1).ext`.

## Checking a device's identity

Both devices show the same 6-digit **verification number** while
connected. If they differ, someone may be between the devices: decline the
transfer. Each device's ID (`k3xq-7m2d-p9ra`) is shown in Settings and on
incoming requests.

## Android in this build

The Android app builds and can send and receive with codes, invitations and
addresses, but finding nearby devices automatically needs a native
component that the current Native Shell packaging cannot include yet. Use a
code or an invitation.

## Reporting a problem

Settings → *Export diagnostics* produces a short report with no file names,
paths, contents or keys. Include it with the steps you took.
