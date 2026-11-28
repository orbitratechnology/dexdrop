# Connectivity: links, speed and fallbacks

What a peer-to-peer link survey (Wi-Fi Direct, LAN, SoftAP hotspots at
5 and 2.4 GHz, Bluetooth Classic, BLE) means for DexDrop: what each
platform lets an app do, what to build, in what order, and what to
expect. The survey ranks Wi-Fi Direct first. That holds for Android but
not for a cross-platform app: iOS and macOS expose no Wi-Fi Direct, and
Windows and Linux expose it unevenly. DexDrop's answer is therefore not
"Wi-Fi Direct first". It is the survey's own conclusion: a
transport-independent engine (which DexDrop already has) with more ways
to get two devices an IP path, picked adaptively.

The speeds below are typical figures for each link, not measurements
of DexDrop. Real-device numbers are still owed (STATUS, "Benchmarks").

## 1. Where DexDrop is today

| Layer | Today | Survey's advice |
|---|---|---|
| Discovery | mDNS/DNS-SD: Bonjour on Apple, NsdManager on Android, mdns-sd elsewhere; QR invitations; 8-character SPAKE2 codes | BLE + mDNS + native peer discovery + QR |
| Links | The shared LAN; across networks Iroh (QUIC hole punching, then a relay) | Wi-Fi Direct, LAN, hotspot 5 GHz then 2.4 GHz |
| Session | Mutual TLS 1.3, pinned Ed25519 identities, consent, SPAKE2 codes | Same |
| Transfer | TCP, 4 MiB chunks, BLAKE3 per chunk and file, chunk-level resume, pipelined disk/hash/TLS (0.9–1.2 GB/s on a RAM disk) | TCP, chunked, TLS 1.3, BLAKE3 or SHA-256, resume |
| Transport independence | `tls::BoxLink`: TCP or an Iroh stream under the same TLS session | "Keep the transfer engine independent of the link" |

So the session and transfer layers already match the survey. The gaps
are all in the **link layer**:

- no link when devices share no network, other than going through the
  internet;
- no measurement of the link before or while it is used;
- one weakness in connecting: a sender tries a device's addresses **one
  after another, with a 4 s timeout each** (`outgoing.rs`, `connect_one`).
  A dead address (a VPN or Docker interface, a stale IPv6 address, a
  second Wi-Fi) costs 4 s before the right one is tried.

The transfer engine needs no change for any link below. Each one ends in
an IP address, and TCP + TLS 1.3 runs over it as it does today.

## 2. What each platform allows

| Capability | Android | iOS | macOS | Windows | Linux |
|---|---|---|---|---|---|
| Wi-Fi Direct (P2P group) | Yes: `WifiP2pManager`. Android 10+ can create a group with a chosen name, passphrase and band (`WifiP2pConfig.Builder`, `setGroupOperatingBand` 5 GHz / 2.4 GHz) | No public API | No public API | Yes: `Windows.Devices.WiFiDirect` (WinRT) | `wpa_supplicant` P2P / NetworkManager `wifi-p2p`, usually needing privileges |
| Host an access point others join | Yes: a Wi-Fi Direct group's credentials let ordinary Wi-Fi clients join it as an access point ("DIRECT-xx"); also `LocalOnlyHotspot` (system picks the SSID and band) | No | No (Internet Sharing is not programmable) | Yes: `WiFiDirectAdvertisementPublisher` with `LegacySettings` (SSID, passphrase) is a soft AP ordinary clients join | Yes: NetworkManager hotspot (`nmcli`, D-Bus) |
| Join a given network from the app | Android 10+: `WifiNetworkSpecifier` (a system confirmation, an app-bound network the sockets must be bound to) | `NEHotspotConfiguration` (a system prompt; "Hotspot Configuration" entitlement) | CoreWLAN `associate` | WLAN API (`WlanConnect`) | NetworkManager |
| Apple peer-to-peer Wi-Fi (AWDL) | No | Yes: Bonjour with peer-to-peer included (`kDNSServiceFlagsIncludeP2P` / `includePeerToPeer`) | Yes, the same | No | No |
| BLE advertise + scan + GATT | Yes (`BLUETOOTH_SCAN`/`ADVERTISE`/`CONNECT`, Android 12+) | Yes (CoreBluetooth; limited advertising in the background) | Yes (CoreBluetooth) | Yes (WinRT advertisement publisher and watcher) | Yes (BlueZ, the `bluer` crate) |
| Bluetooth Classic data | Yes | No (MFi only) | Yes | Yes | Yes |

NSH has no API for any of these (its hardware docs: an app's own crate
reaches the system's frameworks). They are engine-side bridges, as
DexDrop already builds:

- Java bridges in `crates/dexdrop-net/android/`, like `NsdBridge` and
  `AssetsBridge`;
- `objc2` on Apple, like `dexdrop-photos` and `bonjour.rs`;
- the `windows` crate on Windows;
- `zbus` (already in the lock file) or `bluer` on Linux.

## 3. What DexDrop can link, pair by pair

L = same LAN (today), A = AWDL, D = Wi-Fi Direct, H = one device hosts an
access point and the other joins it, I = Iroh (today).

| | Android | iPhone | Mac | Windows | Linux |
|---|---|---|---|---|---|
| **Android** | L, D (5 GHz → 2.4 GHz), I | L, H (Android hosts, iPhone joins), I | L, H (Android hosts), I | L, D or H (either hosts), I | L, H (either hosts), I |
| **iPhone** | | L, **A**, I | L, **A**, I | L, H (Windows hosts), I | L, H (Linux hosts), I |
| **Mac** | | | L, **A**, I | L, H (Windows hosts), I | L, H (Linux hosts), I |
| **Windows** | | | | L, D, I | L, H, I |
| **Linux** | | | | | L, H, I |

Every pair gets a nearby link without a shared network. The only
exception is a desktop and a phone where neither can host. For example,
a Mac and an iPhone use AWDL, and a Mac with a Linux machine that cannot
host falls back to Iroh.

## 4. What to build

### 4.1 Quick wins on the links DexDrop has

1. **Race the addresses (Happy Eyeballs, RFC 8305). Done.**
   - `outgoing::race_tcp` dials a device's addresses together, each one
     250 ms after the one before, or as soon as an earlier one fails, and
     keeps the first TCP connection.
   - Addresses on one of this device's own networks (the same /24 or /64)
     go first, then IPv4, then IPv6, link-local last (`outgoing::order`).
   - The TLS handshake, with any pairing proof, then runs on that one
     connection only, so a code is never spent on a connection that gets
     dropped.
   - A dead address now costs at most the 250 ms stagger, not its 4 s
     timeout (test: `a_dead_address_does_not_hold_up_the_live_one`).
   - **The LAN and the internet race too**, when no pairing proof is
     involved: the internet route (Iroh) starts 1.5 s after the LAN
     (`LAN_HEAD_START`), not after every LAN address has failed, and the
     first connection made wins.
   - Not possible: putting Ethernet first, since a peer's announced
     addresses don't say what kind of link they are on.
2. **Keep Android's Wi-Fi awake during a transfer. Done.**
   - `TransferService` holds `WIFI_MODE_FULL_LOW_LATENCY` (Android 10+)
     and, before Android 14, `WIFI_MODE_FULL_HIGH_PERF` from the first
     progress until no transfer moves.
   - It needs `WAKE_LOCK` (no prompt), which CI's APK check requires.
   - The low-latency lock works while the app is in front with the screen
     on. Before Android 14 the high-performance lock also keeps Wi-Fi out
     of power save in the background.
3. **Measure, then decide.** The CLI now prints each finished transfer's
   measurement: `1.00 GB in 21s, 48.3 MB/s, on this network`.
   - **Receiver:** `dexdrop receive` on one device.
   - **Sender:** `dexdrop send <file> --to <device>` on the other device,
     with a 1 GB file.
   - Do it in both directions, for each pair and link: Wi-Fi 5 or 6
     through a router, Ethernet, and the same Wi-Fi at 2.4 GHz.
   - Record the numbers in `docs/ARCHITECTURE.md` ("Measured
     performance"). The lossy-Wi-Fi comparison of TCP against QUIC is
     still owed too (ADR-001).
4. **Say when a nearby transfer is slow. Done.**
   - The transfer page shows a hint (`id:slow-link`): "This Wi-Fi is
     slow. Both devices on 5 GHz Wi-Fi, nearer the router, or on a cable
     go faster."
   - It sits below Pause and Stop, so showing it never moves them
     (run 121: above them, it pushed Pause under the tab bar of a
     320 × 640 phone just as the emulator check tapped it).
   - It appears when a transfer on this network stays under 5 MB/s once
     its rate is steady, with 10 s or more left (`transfer::slow_nearby`).
   - The route was already shown (on this network, over the internet,
     through a relay); it now re-renders only when the route changes.
5. **Detect client isolation. Open.** It needs the Wi-Fi network's name
   on both devices, which takes the location permission on Android and
   iOS. It is better done with the direct link (4.3), which asks for
   nearby-device permissions anyway.

### 4.2 Apple ⇄ Apple without a shared network: AWDL

Bonjour can browse, advertise and resolve over Apple's peer-to-peer
Wi-Fi.

**Built (step 3); owed: a run on two devices with no network in
common.**

- Every `dns_sd` call (browse, register, resolve, address lookup)
  passes `kDNSServiceFlagsIncludeP2P` (`discovery/bonjour.rs`).
- The listener sets `SO_RECV_ANYIF` before it listens
  (`link::allow_peer_to_peer`, `engine::bind_listener`); a test on the
  macOS runners reads it back.
- A connection whose address is scoped to `awdl0` is reported as
  `Route::PeerToPeer`, shown as "Directly over Wi-Fi" (the CLI says
  "directly over Wi-Fi"); every other local connection stays `Nearby`.
- Address racing tries link-local addresses last, so when the devices
  share a network as well, that network is used and AWDL is the fallback.
- Cost: browsing with peer-to-peer included keeps AWDL active while
  DexDrop runs, which can take a little from the Wi-Fi network's
  throughput, as an open AirDrop window does. DexDrop runs only while its
  window is open on a Mac (ADR-006) and in the foreground on iOS.
- CI has no Wi-Fi on its Macs, so it checks the flags, the socket option
  and the route, not a transfer over AWDL.

- **How:**
  - Set `kDNSServiceFlagsIncludeP2P` on DexDrop's existing `dns_sd`
    browse, register and resolve calls (`bonjour.rs`).
  - Make the listening socket accept connections on the peer-to-peer
    interface (`awdl0`). With BSD sockets, Apple's guidance is the
    `SO_RECV_ANYIF` socket option; confirm this on devices.
  - The rest (TCP + TLS) is unchanged.
- **Expected:**
  - iPhone ⇄ iPhone, iPhone ⇄ Mac and Mac ⇄ Mac transfers with no
    shared Wi-Fi and no prompt beyond the local-network permission the
    app already asks for.
  - Speeds in AirDrop's range, tens of MB/s.
  - It also covers Apple devices on a network that isolates its clients.
- **Effort:** small to medium. It is the most coverage for the least
  work.

### 4.3 The direct link: Wi-Fi Direct, and one device hosting an access point

One mechanism serves every pair in section 3: **one device hosts a
WPA2 network with credentials chosen for this transfer, and the other
joins it.**

**Phase 1 built: the engine's side, on every platform.** What each
platform's radio does comes next (see "Phases" at the end of this
section).

- `direct::DirectLink` (`host`, `stop`, `join`; `can_host`, `can_join`)
  is what a platform implements; the engine owns it
  (`EngineConfig::direct_link`) and calls it on blocking threads.
  `direct::Loopback` stands in for a radio in tests.
- The receiver hosts: Receive shows "Not on the same Wi-Fi? Start a
  direct link" where the device can host one (`Command::HostDirectLink`,
  `Status::direct`). Once the network is up, a new invitation carries it
  (`Invitation::direct`: name, passphrase, band, the host's address), so
  the QR code the sender scans holds everything needed to join.
- Invitations stay readable across versions: an older version reads one
  with a direct link and ignores the link, and this one reads older ones.
- The sender tries, one at a time: the network in common (there may be
  one after all), then joins the hosted network and connects through
  what the join returned (`Joined::prepare`, where Android will bind the
  socket), then the internet. The joined network is kept for the
  transfer's reconnections and left when it ends.
- Both ends report the route as `PeerToPeer`, "Directly over Wi-Fi": the
  receiver recognizes devices that joined its network by their address.
- Hosting stops when the user stops it or leaves Receive, but never under
  a transfer that uses it: then it stops when that transfer ends.
- Tests: `tests/direct.rs` in the engine (a transfer over the hosted
  network with nothing else in common; hosting outliving the invitation
  while a transfer runs; a network that cannot start says why; nothing
  without a link) and in the app (Receive's states, pictures in
  `target/shots/direct`).

- **The host:**
  - Android creates a Wi-Fi Direct group with `WifiP2pConfig.Builder`:
    a random SSID (`DIRECT-xx-DexDrop`) and passphrase, band 5 GHz first
    and 2.4 GHz when 5 GHz is refused. It is the survey's "5 GHz hotspot,
    then 2.4 GHz", done in a way apps may use. (`LocalOnlyHotspot` lets
    the system choose the band, so it is the last resort.)
  - Windows hosts with the Wi-Fi Direct publisher in legacy mode, and
    Linux with a NetworkManager hotspot.
- **The joiner:**
  - Android ⇄ Android and Windows ⇄ Windows join as Wi-Fi Direct
    clients with the credentials. On Android 10+ this should need no
    invitation dialog; confirm on devices.
  - Every other platform joins as an ordinary Wi-Fi client:
    - iPhone: `NEHotspotConfiguration`, with one system prompt;
    - Mac: CoreWLAN;
    - Windows: WLAN API;
    - Linux: NetworkManager;
    - Android: `WifiNetworkSpecifier`, with one system prompt.
- **Getting the credentials across:**
  - Today: a QR code or invitation. Extend the invitation with `ssid`,
    `psk`, `band` and the host's address.
  - With 4.4: over BLE, automatically.
  - Either way the credentials are short-lived, are used for one
    transfer, and travel only to a device that then proves its identity
    with TLS.
- **Binding:**
  - On Android a joined network is app-bound and has no internet. The
    engine's sockets must be bound to it (`Network.bindSocket`, through
    the Java bridge).
  - Desktops may need the route pinned to that interface.
- **Permissions, asked only when the user first uses a direct link:**
  - Android: `NEARBY_WIFI_DEVICES` (13+, `neverForLocation`), or location
    up to Android 12 with Location turned on.
  - iOS: the Hotspot Configuration entitlement.
  - macOS: location permission may be needed for Wi-Fi scanning.
  - Windows and Linux: none for the user, though Linux may need polkit
    rules.
- **Cost:**
  - A phone that hosts or joins may leave its Wi-Fi network for the
    duration on devices without simultaneous client and AP or P2P
    support. Say so before switching, and switch back afterwards.
- **Expected:**
  - 5 GHz phone ⇄ phone: typically 20–60 MB/s.
  - 5 GHz with Wi-Fi 6 hardware: more.
  - 2.4 GHz fallback: 3–10 MB/s.
  - Compared with the same two devices both on one Wi-Fi router, a
    direct link can be up to about twice as fast, because each frame
    crosses the air once instead of twice. It can also be slower when a
    phone shares one radio between its router and the group on different
    channels. This is why 4.5 measures before it switches.
- **Effort:** medium to large. It is one module per platform behind one
  `DirectLink` trait (`host()`, `join(credentials)`, `close()`), owned by
  the engine like discovery.
- **Phases:**
  1. **Done:** the engine, the invitation, Receive, tests (above).
  2. **Built; owed: a run on two phones.** Android
     (`direct/android.rs`, `DirectLinkBridge.java`):
     - Hosting: a Wi-Fi Direct group with DexDrop's name, passphrase and
       band (`WifiP2pConfig.Builder`, Android 10+), 5 GHz first and
       2.4 GHz when 5 GHz is refused; before Android 10 the system
       chooses all three. The group's owner address (192.168.49.1,
       a /24) is the invitation's host.
     - Joining (Android 10+): `WifiNetworkSpecifier` with the name and
       passphrase; the system asks the user once. The engine's sockets to
       the host are bound to that network with the NDK's
       `android_setsocknetwork`, as a joined network has no internet and
       is not the default; leaving releases the request.
     - Permission: hosting asks for Nearby devices
       (`NEARBY_WIFI_DEVICES`, Android 13+) or fine location (Android 12
       and earlier) the first time; a refusal says so. NSH's manifest
       keys take permission names only, so the two with attributes
       (`neverForLocation`; location only up to Android 12) come from
       `android/direct-link.aar`'s manifest; CI's APK check reads both
       attributes back.
     - CI: the emulator check starts a direct link where its image offers
       Wi-Fi Direct and expects it on or a reason, with no crash. The
       Android 14 emulator hosts it (run 125: "Direct link on (2.4 GHz)",
       5 GHz refused there and 2.4 GHz taken), and CI's APK check found
       `neverForLocation` and the location cap in the packaged manifest.
       A transfer over it needs two phones.
     - Android is the host for most pairs in section 3, and phone ⇄
       phone needs nothing else.
  3. **Linux built; owed: a run on a computer with Wi-Fi.** Linux
     (`direct/linux.rs`), through NetworkManager on the system bus:
     hosting adds a WPA2 access point (`mode ap`, 5 GHz as `band a`
     where the card can, else 2.4 GHz) with shared IPv4, so
     NetworkManager's DHCP gives joiners addresses and the host is
     10.42.0.1/24; joining adds an ordinary client connection. Both are
     made for the transfer and deleted after, never autoconnecting. A
     computer with one radio leaves its own Wi-Fi meanwhile and
     NetworkManager returns to it. Desktop sessions may change networks
     without a password (NetworkManager's polkit defaults); a refusal is
     said. Offered only where NetworkManager has a Wi-Fi device (an access
     point needs its AP capability). **Windows** (the Wi-Fi Direct
     publisher in legacy mode, the WLAN API to join) is next.
  4. Joining on Apple devices: `NEHotspotConfiguration` on iOS (its
     entitlement and one system prompt), CoreWLAN on a Mac.
  5. Hosting from the sender's side too, for pairs where only the sender
     can host (an iPhone receiving from an Android phone): with BLE (4.4)
     or a QR code the sender shows.

### 4.4 BLE: finding each other and handing over credentials

BLE does what the survey assigns it, and nothing more.

- **Advertising:**
  - A DexDrop service UUID and a short rotating ID.
  - No names: the same privacy rule as the mDNS TXT records.
- **Exchange:** a GATT characteristic carries what the invitation would:
  - the device's key fingerprint;
  - its supported links;
  - when asked, a direct-link offer (4.3).
- **What it adds:**
  - Devices find each other with no shared network and no QR code. A
    nearby device appears in Nearby as it does on a LAN today.
  - The direct link is then set up automatically.
- **Limits:**
  - iOS limits advertising in the background (the UUID moves to an
    overflow area that other iOS devices scan only in the foreground).
    Expect discovery while DexDrop is open, which is how it is used
    anyway.
  - Android 12+ needs the Bluetooth permissions above.
  - Desktops without Bluetooth keep using mDNS and QR.
- **Effort:** medium to large. Bridges: Android (Java), Apple
  (CoreBluetooth, `objc2`), Windows (WinRT), Linux (`bluer`).

### 4.5 An adaptive link manager, and upgrades in the middle of a transfer

The engine already picks among links (LAN, then Iroh). Grow that into a
planner.

1. **Candidates.**
   - LAN addresses, raced (4.1).
   - AWDL (Apple).
   - A direct link, when the other device is nearby: seen over BLE, or
     with a direct-link offer in its invitation.
   - Iroh direct, then the Iroh relay.
2. **The order.**
   - The same LAN, then AWDL, then the direct link, then Iroh direct,
     then the relay.
   - **A device known to be nearby is always offered a direct link
     before the internet.** Iroh between neighbours runs through their
     internet uplinks, often 5–50 Mbit/s, may cost mobile data (FR-072
     asks first), and the relay is rate-limited.
3. **Upgrade.**
   - Start on whatever connects first. After a few seconds, if the
     measured goodput is low (for example under 10 MB/s) and a faster
     link is possible, set it up beside the running transfer.
   - Then pause, switch and resume. Chunk-level resume makes the switch
     cost under one 4 MiB chunk per file.
   - This is Nearby Connections' "bandwidth upgrade", built on what
     DexDrop already has.
4. **Fallback.**
   - When a direct link fails or drops, resume on the LAN or the
     internet, and the reverse.
   - The user sees the route change: "Switched to a direct Wi-Fi link,
     48 MB/s".

### 4.6 Faster on any link, if the numbers call for it

- **Parallel TCP streams per transfer** (2–4 connections taking chunks
  from one manifest). A single TCP flow often leaves lossy Wi-Fi
  underused.
  - Expected: 10–40% on poor Wi-Fi; nothing on a clean link.
  - Do it only if 4.1's measurements show a gain (STATUS lists it as
    partial, pending exactly that).
- **Adaptive limits:** chunks in flight and requests outstanding from
  the measured RTT and goodput (STATUS: "Adaptive concurrency: Open").
- **Socket buffers:** larger send and receive buffers matter only on
  high-latency paths (Iroh direct). Leave the LAN to the OS's
  autotuning.

### 4.7 What not to build

- **Bluetooth as a data link** (Classic or BLE L2CAP).
  - BLE moves about 0.05–0.2 MB/s, so a 100 MB video takes 10 minutes
    or more.
  - iOS has no Classic Bluetooth for apps.
  - At most, a later "send text or a link over Bluetooth" for when
    nothing else works.
- **Replacing TCP with QUIC on the LAN** without the lossy-Wi-Fi
  measurement (ADR-001). Iroh keeps QUIC where it matters: across
  networks.
- **Interoperating with Quick Share or AirDrop.** Both are closed
  protocols.

## 5. Order and expectations

| Step | Builds | Platforms | Effort | Expected improvement |
|---|---|---|---|---|
| 1 | **Done:** address racing + LAN/Iroh race (4.1.1), Android WifiLock (4.1.2) | All, Android | Days | Setup from seconds to under one when a device has several addresses; steadier and often faster Android transfers |
| 2 | **Done:** the CLI's measurement line and the slow-Wi-Fi hint. Owed: the benchmark matrix on real devices (4.1.3) and the isolation hint (4.1.5) | All | Days | Numbers for every later decision; users told why a transfer is slow |
| 3 | **Built:** AWDL (4.2). Owed: a run on two Apple devices with no shared network | iOS, macOS | 1–2 weeks | Apple devices transfer with no shared network, at AirDrop-like speeds |
| 4 | Direct link through QR or invitation (4.3). **Phases 1 and 2 built, Linux of 3:** the engine, the invitation, Receive, tests; Android hosting (5 GHz, then 2.4 GHz) and joining (owed: two phones); Linux through NetworkManager (owed: a computer with Wi-Fi). Next: Windows, then the Apple joiners | All | 3–5 weeks | Every pair in section 3 works with no shared network or internet, typically 20–60 MB/s at 5 GHz; no mobile data spent between nearby devices |
| 5 | BLE discovery + credential exchange (4.4) | All with Bluetooth | 3–4 weeks | Nearby devices appear and connect with no network and no QR code |
| 6 | Link manager with upgrades in the middle of a transfer (4.5), parallel streams if measured (4.6) | All | 2–3 weeks | The best measured link, chosen and switched automatically; up to about 2× over a router for two wireless devices |

## 6. The resulting fallback chain

```text
Discovery: mDNS/Bonjour (+AWDL on Apple) ─┐
           BLE (nearby, no network)       ├─► candidates for the device
           QR / invitation / code         ─┘

Connect, racing within each step:
  1. Same LAN: every address raced, Ethernet first
  2. Apple ⇄ Apple: AWDL
  3. Nearby: a direct link: Wi-Fi Direct group 5 GHz → 2.4 GHz,
     or a desktop-hosted access point (credentials over BLE or the QR code)
  4. Different places: Iroh direct (hole punching)
  5. Iroh relay (rate-limited; always shown)

While transferring: measure goodput; upgrade to a faster link when one is
possible (resume makes the switch cheap); on a drop, resume over the
next link down.
```

The session (mutual TLS 1.3, pinned identities, SPAKE2 codes, consent)
and the transfer (chunks, BLAKE3, resume) stay the same on every link.
That is what makes the fallbacks safe to add.
