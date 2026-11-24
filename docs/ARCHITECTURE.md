# Architecture

```text
┌──────────────────────────── app/ (NSH, Rust only) ───────────────────────────┐
│ shell (sidebar ⇄ tab bar) · pages (home, send, receive, transfer, …) · ui     │
│ state: global signals  ◀── events ──  bridge  ── commands ──▶                 │
└──────────────────────────────────────────────┬───────────────────────────────┘
                                               │ dexdrop_core::api (Command/Event)
┌──────────────── crates/dexdrop-net (own thread + Tokio runtime) ─────────────┐
│ engine actor: transfers, consent, trust, history, progress sampling (4 Hz)   │
│ discovery (mDNS)   listener → incoming   outgoing (connect/retry/resume)      │
│ session: pipelined requests, chunk streaming, verification, journal          │
│ tls: mutual TLS 1.3, self-signed Ed25519 certs, peer ID pinning, SAS         │
└──────────────────────────────────────────────┬───────────────────────────────┘
┌──────────────── crates/dexdrop-core (no I/O runtime, no UI) ─────────────────┐
│ identity · protocol · pathsafe · manifest · naming · journal · invitation    │
│ store · format · api                                                          │
└───────────────────────────────────────────────────────────────────────────────┘
```

Dependencies only point down. The core compiles to `wasm32-unknown-unknown`
(native-only pieces are `cfg`-gated) so the same app crate builds the web
preview, where the engine is absent.

## ADR-001: Transport for the MVP — TCP + TLS 1.3

**Status:** accepted for the local-network MVP; revisit with the remote mode.

The PRD lists Iroh/QUIC as the leading candidate subject to benchmarks. For
the local-only MVP we chose Tokio TCP + rustls TLS 1.3:

- It builds on every target NSH supports today, including Android, iOS and
  Intel Macs, with no extra platform matrix to validate.
- Same-LAN transfers need no NAT traversal or relays, which is Iroh's main
  advantage; the local-only policy forbids relays anyway.
- Measured: 400–500 MB/s end to end on loopback (TLS + BLAKE3 + disk writes),
  far above 1 GbE and Wi-Fi rates, so the transport is not the bottleneck.

The engine keeps transport behind `tls::connect` / `tls::accept` and generic
`AsyncRead + AsyncWrite` sessions; the session code is transport-agnostic (its
adversarial tests run over in-memory pipes). Adding Iroh for remote mode means
providing another stream source plus its peer identity, not changing sessions.

### Benchmark: TCP + TLS 1.3 against QUIC (P0-012)

`cargo run --release -p dexdrop-bench -- 1024 5` sends 1 GiB one way, five
interleaved rounds, over each transport with the same Ed25519 identities,
rustls (ring) and TLS 1.3, one stream, no hashing or disk:

| Transport | Median throughput | CPU per GB, both ends |
|---|---|---|
| TCP + TLS 1.3 (tokio-rustls) | 1,747 MB/s | 0.99 s |
| QUIC (quinn 0.11, the QUIC stack under Iroh) | 519 MB/s | 4.48 s |

On this machine TCP + TLS moves data about 3.4x faster at about a quarter of
the CPU per GB, which also matters for battery on phones. Caveats:
loopback has no loss or latency, where QUIC's loss recovery and lack of
head-of-line blocking help; QUIC can use UDP segmentation offload on real
network cards, which loopback in this container does not exercise; and
QUIC's real advantage, NAT traversal and connection migration for remote
mode, is out of scope for local transfers. The container has no `netem`,
so the lossy Wi-Fi comparison still has to be run on real hardware with
the same bench. The decision stands for the local-network MVP.

## ADR-002: Transfers over the internet — Iroh beside TCP

**Status:** accepted (Phase 6, PRD 6.8 and 9.5).

Same-network transfers keep TCP + TLS 1.3 (ADR-001). Devices on different
networks connect through [Iroh](https://www.iroh.computer) 1.5: QUIC
connections dialed by public key, with address lookup, NAT traversal (hole
punching, port mapping) and a relay fallback when no direct path exists.

- **One key.** Iroh's key is the device's own Ed25519 key (its seed, read
  from the identity's PKCS#8), so a device's internet address is the public
  key in its certificate (`ed25519_key_of_cert`). `PeerId::from_ed25519_key`
  maps it back: a dialed key is checked against the expected peer before
  dialing, and an incoming Iroh key must match the TLS certificate.
- **Same protocol.** DexDrop's mutual TLS runs inside one Iroh
  bidirectional stream exactly as over TCP (`tls::BoxLink` takes either), so
  pinning, the verification number, SPAKE2 code proofs, consent, resume and
  verification are unchanged. Iroh's own encryption is a second layer; at
  internet speeds the extra CPU does not matter.
- **Order.** Outgoing transfers try the local network first (discovered
  addresses, invitation addresses), then the internet when the other
  device's key is known: from an invitation (which now carries the key and
  home relay) or from `known-devices.json`, the devices this one exchanged
  files with. Codes stay local: a typed code has no room for a key, and
  looking codes up remotely would need a rendezvous service.
- **Route.** Each transfer reports `Nearby`, `Direct` or `Relayed`, and
  follows Iroh moving from the relay to a direct path; the UI always says
  when a relay carries the data (PRD 9.5).
- **Relays.** n0's public relays by default, which are rate-limited and have
  no SLA (development); `DEXDROP_RELAY_URL` or the CLI's `--relay` points at
  the project's own `iroh-relay` for production. Address lookup publishes
  and resolves endpoint addresses through n0's DNS (pkarr).
- **Off switch.** Settings › "Send over the internet" (on by default) and the
  CLI's `--local-only`; local transfers never use a relay either way.
- **Tests.** `tests/internet.rs` (feature `test-relay`, on in CI) runs two
  engines with local discovery off against a local relay: an invitation,
  then a known device by ID alone.

## ADR-003: Sending to several devices — parallel unicast, one reading

**Decision.** The same selection goes to several devices as one
independent transfer per device, all started at once: each its own TLS 1.3
session (or Iroh connection), its own consent and verification number,
journal, pause, resume and retry. The selection is read once (one
`Manifest`, shared as `Arc`) for every device of a group
(`Command::Send { targets, paths, group }`); devices tapped later in the
same group reuse that reading. Receiving from several devices at once
already worked (up to 32 incoming connections).

**Why not something else.**

| Option | Verdict |
|---|---|
| Wi-Fi multicast / broadcast | Rejected. 802.11 sends multicast at the lowest basic rate, without acknowledgements or retransmission, so it is slower than unicast and lossy (needs FEC); many access points and phones filter or rate-limit it; iOS needs the multicast entitlement; no per-device TLS or consent. |
| Recipients relaying to each other (swarm, as BitTorrent or iroh-blobs) | Later, if measured. It helps only when the sender's own link is the bottleneck and the recipients have spare capacity, needs protocol v2, and has recipients trust each other's chunks (BLAKE3 per chunk makes that verifiable). |
| One after another | Rejected: N times as long, and the last device waits for every other. |
| Parallel unicast | Chosen: no protocol change, the existing pipeline per device, each device as fast as its link allows, one slow or declining device holding up no other. |

**Costs, measured.** The sender reads each file once per device (the page
cache serves the repeats while transfers run in step) and hashes and
encrypts per device. `dexdrop bench --to N` sends 512 MB from one engine to
N engines, all on one 4-core machine (the receivers share its CPU and disk,
so each device's figure is lower than on its own hardware):

| Receivers | Each | All together |
|---|---|---|
| 1 | 534 MB/s | 534 MB/s |
| 2 | 266–294 MB/s | 532–587 MB/s |
| 4 | 167–174 MB/s | 670–678 MB/s |

CI's runners, `bench --mb 512` to one device then `--mb 256 --to 3` (run 71):

| Runner | To one | To three at once, in all |
|---|---|---|
| Linux (4 vCPU) | 161 MB/s | 198 MB/s |
| macOS 15 (arm64) | 345 MB/s | 473 MB/s |
| Windows (slow shared disk) | 52 MB/s | 75 MB/s |

Total throughput grows with receivers: the sender is not the limit at
these rates, let alone at Wi-Fi's (tens of MB/s). On one Wi-Fi network the
devices share its airtime, so N devices each get about 1/N of what one
would; wired or separate bands do not.

**Experience.** Tap a device and it is sent to at once; the list stays and
each row shows its transfer (waiting, percent, sent, declined); tap another
to send to it too; "Send to all N nearby" sends to every nearby device in
one tap. "See progress" shows all of them with an overall bar and Pause
all / Resume all. One tap per device, no selection mode (the AirDrop and
Quick Share pattern). The CLI takes several recipients: `dexdrop send
"Desk,Laptop" file`.

## Measured performance

`dexdrop bench` runs two engines in one process over loopback TLS, including
hashing and disk writes (build container, release build):

| Workload | Result |
|---|---|
| One 1 GiB file | 340–480 MB/s |
| 300 files of 3 MB (photos) | 624 MB/s |
| 2,000 files of 16 KiB | 3,100–3,500 files/s, 17 MB peak memory |
| 20,000 files of 16 KiB | 3,700 files/s, 40 MB peak memory |

Large files are pipelined: on the sender a reader thread reads and hashes
the next chunks (at most 2 ahead) while the session encrypts and sends;
on the receiver a writer thread per file verifies, hashes and writes
while the session reads the next chunk off the network (at most 2
behind), and journals only what it has verified on disk. Disk, BLAKE3 and
TLS used to take turns on about one core of four. Measured on a RAM disk
(storage out of the way): 0.34–0.55 → 0.93–1.19 GB/s; a 5.37 GB file in
4.5 s with 51 MB peak memory for both engines. On this container's disk
both run at its ~500 MB/s. The wire protocol is unchanged. Chunk sizes of
1, 4 and 16 MiB measure the same; 4 MiB keeps resume losses small.

Small files were first limited by thread hand-offs (each async file
operation hops to a blocking thread) and then by one fsync per file. Files
that fit in one chunk are now read whole on the sender and written, synced
and committed in one blocking call on the receiver, with up to 8 commits in
flight while the next files arrive: 1,400 → 3,700 files/s.

## Large files (5 GB+)

What the usual advice for large transfers over TCP + TLS asks, and where
DexDrop stands:

| Advice | DexDrop | Why |
|---|---|---|
| Fixed chunks of 4–16 MB | 4 MiB (1–16 MiB negotiable) | 1, 4 and 16 MiB measure the same; 4 MiB keeps what a drop loses small |
| Never hold whole files in memory | Chunks are streamed from and to disk; at most 2 chunks wait per side; 51 MB peak for a 5.37 GB file (both engines). Phones pick through `pick_paths`, which copies to storage instead of memory | |
| Overlap disk, hashing and the network | Reader and writer threads beside the TLS session | Measured 2× |
| Chunk-level resume and hashing | BLAKE3 per chunk and per file; the verified prefix is journaled every 2 s; the sender sends only what is missing | BLAKE3 is fast enough (several GB/s per core with SIMD) and, unlike XXH3, cryptographic |
| Zero-copy (mmap, `sendfile`, `TransmitFile`) | Not used | TLS encrypts in the process, so the kernel cannot send file pages directly (kernel TLS exists on Linux only); Rust has no garbage collector, so reading into reused buffers costs no pauses; `mmap` turns a file truncated while sending into a crash (`SIGBUS`) |
| `SO_SNDBUF` / `SO_RCVBUF` of 4–8 MB | Left to the system | Setting them turns Linux's and Android's automatic tuning off and is capped by `rmem_max` (often 208 KB): slower, not faster. macOS and Windows tune up to 4–16 MB on their own, and a LAN's bandwidth × delay is ~250 KB at 1 Gbit/s and 2 ms |
| 4–8 parallel TLS connections | One connection per transfer | One TCP flow fills a LAN when its window is not starved; parallel flows help on lossy or long links and when one core cannot encrypt fast enough. Planned behind a measurement on real Wi-Fi 6/6E/7: chunks striped over N connections need a chunk bitmap for resume and writes at offsets (protocol v2), a large change to take only for a measured gain |

## Identity and authentication

Each install generates an Ed25519 key and a self-signed certificate. Peer ID =
BLAKE3 (derive-key mode) of the certificate's SubjectPublicKeyInfo. Both sides
present certificates (mutual TLS); signatures are verified normally, so a peer
must hold the private key of the ID it claims. Callers pin the expected ID
(from discovery, an invitation or a stored record) and fail closed on mismatch.
Both UIs show a 6-digit verification number derived from TLS exporter keying
material, which differs if anyone is in the middle.

The private key goes to the system's credential store where one works
without prompts (Windows Credential Manager, Linux Secret Service, iOS
Keychain) and otherwise stays in the app-private identity file; where it is,
why, and the threat model: [SECURITY.md](SECURITY.md).

## Code pairing (SPAKE2)

A typed code is 40 bits, too short to send to whoever claims to show one or
to expose to offline guessing. The sender never sends it: both sides run
SPAKE2 (`spake2` crate, Ed25519 group) with the code, and each proves it
derived the same key with a BLAKE3 MAC over TLS exporter material of that
session (`pairing.rs`). The sender only offers files to a device whose proof
verifies; the receiver only counts the sender as invited after the sender's
proof. Every attempt counts against the code from the moment it starts
(5 in total, including ones that hang up), and a proven code is used up.

## Protocol

See `crates/dexdrop-core/src/protocol.rs`. Length-prefixed postcard frames
(≤ 256 KiB, checked before allocation), version negotiation in `Hello`, a
paginated manifest of relative paths, then receiver-driven `Want`/`Skip`
requests with at most 64 outstanding, chunk headers carrying BLAKE3 followed by
raw bytes, and `FileEnd` with the whole-file hash. The receiver acknowledges
each file and sends `Complete`.

Duplicate detection (FR-059) costs nothing unless it can pay off. Only when
the destination already holds a regular file of the same name and size does
the receiver hash its copy and send `Probe`. The sender answers with
`FileHash`, the hash of its source, and the receiver decides: the same, and
the file is kept as it is (`Skip`, counted as received); different, and it
asks with `Want` as usual. The receiver compares, so a sender learns only
whether the receiver took the file, never the hash of the receiver's copy.
Each file can be probed once and only before it is requested. With "keep
both", a folder that already exists is renamed as a whole, so this applies
to files at the top level and to "replace".

## Receiving and resume

Each file is written to a hidden `.<name>.<id>-<n>.dexdrop-part` sibling, so
the final rename is on one filesystem. Every two seconds the receiver flushes
data to disk and then journals the verified, chunk-aligned prefix — never the
other way round. On reconnect, a sender with the same transfer ID and an
identical manifest digest is accepted without asking again, and each file is
requested from its verified offset; the sender re-hashes the prefix so the
whole-file hash still covers it. Verified files are committed with a
no-clobber hard link (rename fallback) for "keep both" and "skip", or a
replacing rename for "replace".

The journal is stored as a snapshot plus an append-only log of the files
that changed since, so a save costs what changed rather than the whole
manifest. Log lines carry the snapshot's generation: lines older than the
snapshot, or cut short by a crash, are ignored on load. The log is folded
into a new snapshot once it has as many lines as the journal has files.

## Discovery

`_dexdrop._tcp` with TXT `v`, `id`, `n`, `p`, `c` (code open) and `a`
(avatar). No file names or account names are advertised. Each platform
uses its own service behind one `DiscoveryEvent` contract
(`crates/dexdrop-net/src/discovery/`):

- **Apple** (iOS, macOS): the system's Bonjour (`dns_sd`), so iOS asks for
  local-network permission once and needs no multicast entitlement
  (`bonjour.rs`). The iOS Simulator uses mdns-sd.
- **Android:** the system's `NsdManager` through a Java bridge loaded from
  memory (`nsd.rs`, Android 12+; older versions fall back to mdns-sd).
- **Windows, Linux:** mdns-sd, a Rust responder that shares port 5353
  with Avahi or Windows' own (`mdns.rs`).

Devices on other networks are not discovered: they are reached by key
(ADR-002), from an invitation or a previous exchange.

## ADR-004: Copies of picked files — the app's selection decides

**Context.** Phones' pickers hand over copies (NSH's `nsh-picker` in the
app's cache), which the app moves into its `staging` folder. A 1 GB video
picked is 1 GB more in the app's storage. The engine used to delete a copy
when its transfer ended, but kept everything the last send group read until
another group replaced it, kept small photos a week for History, and
cleaned up strays only at the next start, an hour later. A user saw the
app's storage grow with every share.

**Decision.** The app tells the engine what is selected to send
(`Command::KeepPicked`) each time the selection changes. The engine keeps a
copy only while the selection, an unfinished (or failed, retryable)
transfer, or the open send group reads it; a send group with another
selection is over. A copy taken out of the selection, and a finished
transfer's copies, go at once; any other copy nothing needs goes when the
selection changes, when a transfer ends and at start, unless it was
written in the last minute (a pick still being copied, or not yet in the
selection). Only DexDrop's `staging` and NSH's picker folders are touched,
never the user's files; a computer's shared temporary folder keeps its
one-hour rule.

**Consequences.** Storage returns to its size once a share is over, and
after a restart at the latest when the app was closed mid-pick. History no
longer shows previews of sent photos (their copies are gone). Tests:
`a_groups_copies_go_when_the_selection_is_cleared`,
`copies_nothing_selects_go`, `copies_of_picked_files_go_once_sent`
(engine), `a_picked_copy_goes_when_taken_out_of_the_selection` (app).

## ADR-005: Send's Recent — the system's media library, sent in place

**Context.** Picking a photo through the system's picker on a phone takes
several taps, and the picker hands over a copy (ADR-004). A user asked for
the latest photos, videos and files right on Send.

**Decision.** Android: a Java bridge (`MediaBridge`, in the same in-memory
dex as the discovery and apps bridges) asks `MediaStore` for the newest
photos and videos and writes small JPEG thumbnails (256 px, kept only for
the items listed). Reading them needs `READ_MEDIA_IMAGES`/`READ_MEDIA_VIDEO`
(Android 13+; Android 14's "limited access" works too, with a button to
choose more) or `READ_EXTERNAL_STORAGE` before; the manifest patch declares
them, and Send asks only when the user taps "Show recent photos". Items are
sent from their own files (`MediaStore`'s path, readable with that
permission): nothing is copied, nothing is cleaned up. Computers list the
newest files directly in Downloads, Pictures and Movies (plus Desktop and
Documents on Windows and Linux; not on macOS, which asks before an app
reads them). iOS has no photo library API in NSH 0.6.1 (`PHPhotoLibrary`
would need native code NSH cannot add yet): no Recent there.

**Consequences.** One tap to send a recent photo on Android, with no copy.
The permission dialog is asked for from the bridge (`requestPermissions` on
the activity in front, found through `ActivityThread`); the answer is read
again when the app is back in front. Tests: `media` unit tests (the
bridge's lines, folder listing, thumbnails kept), `app/tests/recent.rs`,
and the emulator check's `13-recent` (a photo added to the camera folder
shows).
