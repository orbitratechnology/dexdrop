# Architecture

```text
┌──────────────────────────── app/ (NSH, Rust only) ───────────────────────────┐
│ shell (sidebar ⇄ tab bar) · pages (Nearby, Transfers, Settings) · ui tokens   │
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

## Measured performance

`dexdrop bench` runs two engines in one process over loopback TLS, including
hashing and disk writes (build container, release build):

| Workload | Result |
|---|---|
| One 1 GiB file | 340–480 MB/s |
| 300 files of 3 MB (photos) | 624 MB/s |
| 2,000 files of 16 KiB | 3,100–3,500 files/s, 17 MB peak memory |
| 20,000 files of 16 KiB | 3,700 files/s, 40 MB peak memory |

Small files were first limited by thread hand-offs (each async file
operation hops to a blocking thread) and then by one fsync per file. Files
that fit in one chunk are now read whole on the sender and written, synced
and committed in one blocking call on the receiver, with up to 8 commits in
flight while the next files arrive: 1,400 → 3,700 files/s.

## Identity and authentication

Each install generates an Ed25519 key and a self-signed certificate. Peer ID =
BLAKE3 (derive-key mode) of the certificate's SubjectPublicKeyInfo. Both sides
present certificates (mutual TLS); signatures are verified normally, so a peer
must hold the private key of the ID it claims. Callers pin the expected ID
(from discovery, an invitation or a stored record) and fail closed on mismatch.
Both UIs show a 6-digit verification number derived from TLS exporter keying
material, which differs if anyone is in the middle.

The private key is stored in an app-private file (mode 0600 on Unix).
Moving it to the OS keystore (Keychain, Android Keystore, DPAPI, Secret
Service) is open work.

## Protocol

See `crates/dexdrop-core/src/protocol.rs`. Length-prefixed postcard frames
(≤ 256 KiB, checked before allocation), version negotiation in `Hello`, a
paginated manifest of relative paths, then receiver-driven `Want`/`Skip`
requests with at most 64 outstanding, chunk headers carrying BLAKE3 followed by
raw bytes, and `FileEnd` with the whole-file hash. The receiver acknowledges
each file and sends `Complete`.

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

## Discovery

`mdns-sd` advertises `_dexdrop._tcp.local.` with TXT `v`, `id`, `n`, `p`, `c`
(code open). No file names or account names are advertised. The PRD's
platform-specific adapters (Network framework on iOS, `NsdManager` on Android)
plug into the same `DiscoveryEvent` contract; they need native bridges that
standalone NSH packaging does not provide yet.
