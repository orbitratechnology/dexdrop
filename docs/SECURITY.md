# Security notes: identity keys and local data

The transport and pairing design is in [ARCHITECTURE.md](ARCHITECTURE.md).
This page covers what DexDrop keeps on the device and what protects it (PRD
9.9: "Use secure OS storage for identity private keys where available;
otherwise use encrypted app-private storage with a documented threat model").

## The identity key

Each install has one Ed25519 key. Its certificate (public) is in
`identity.json` in the app's state directory; the private key is kept here:

| Platform | Private key in | Protects against | Does not protect against |
|---|---|---|---|
| Windows | Credential Manager (encrypted with DPAPI for the user) | other users of the PC; reading the disk offline | programs the same user runs |
| Linux with a Secret Service (GNOME Keyring, KWallet) | the user's keyring (encrypted with the login password) | other users; reading the disk offline while logged out | programs the same user runs while the keyring is unlocked |
| Linux without one (servers, minimal desktops) | `identity.json`, mode 0600 | other users | root; the same user; reading the disk offline unless the disk is encrypted |
| iOS | Keychain, data-protection store, readable by DexDrop only | other apps; reading a locked device | a jailbroken device |
| macOS | `identity.json`, mode 0600 | other users | the same user; reading the disk offline unless FileVault is on (the default) |
| Android | `identity.json` in the app's private storage | other apps (the Android sandbox); reading a locked device (file-based encryption) | a rooted device |

Why not everywhere yet:

- **macOS**: Keychain items are tied to the app's code signature. DexDrop's
  macOS builds are ad-hoc signed until there is a Developer ID (PRD release
  gate), so every update would get a new signature and macOS would ask the
  user to allow access to the key after each one. The Keychain comes with
  signed releases.
- **Android**: the Android Keystore store (`android-native-keyring-store`)
  finds the Java VM through `ndk-context`, which NSH sets from its next
  release (longbridge/nsh#2686). Until then the key stays in the app's
  private storage, which Android already keeps from other apps and encrypts
  at rest. "Encrypting" that file with a key stored beside it would add
  nothing.

Behaviour:

- A key found in `identity.json` moves to the credential store on the next
  start where the store works; the file keeps its copy until the store has
  confirmed the write by reading it back.
- If `identity.json` says the key is in the store and the store does not
  answer (locked keyring, no session bus), DexDrop reports an error rather
  than replacing the identity. If the store answers that the key is gone
  (deleted by the user), DexDrop creates a new identity: other devices then
  see a new device and must trust it again.
- A key that does not belong to the stored certificate is refused.
- `DEXDROP_KEY_STORE=file` keeps the key in the file on every platform.
- The diagnostics report says where the key is (`identity_key`) and never
  contains the key, file names or paths (checked by the transfer tests).

What an attacker with the key can do: present itself as this device to
devices that trust it, and so send them files (auto-accepted only where the
user turned that on for this device) or receive files they send to it. They
cannot read past transfers: TLS 1.3 sessions use ephemeral keys.

## Other local data

`trusted-devices.json`, `history.json`, `settings.json` and the resume
journals sit next to `identity.json` with the same file protection. They hold
device names, peer IDs, file names, sizes and the download folder, but no
file contents and no secrets: invitation and pairing secrets live only in
memory and expire.
