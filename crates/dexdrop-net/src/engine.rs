//! The transfer engine: one actor that owns discovery, the listener, every
//! transfer and the local stores. The UI sends [`Command`]s and renders the
//! [`Event`]s it gets back; it never touches sockets or files (PRD 9.1).
//!
//! Progress is sampled a few times a second and only changed snapshots are
//! sent, so the UI sees at most ~4 updates per second per transfer (PRD 7.4).

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use dexdrop_core::{
    Identity, PeerId, format,
    invitation::{self, DEFAULT_TTL_SECS, InvitationBook},
    journal::{OutgoingRecord, ReceiveJournal, remove_partials},
    manifest::Manifest,
    naming::ConflictPolicy,
    protocol::{DEFAULT_CHUNK_SIZE, RejectReason, TransferId},
    store::{
        Direction, History, HistoryEntry, Outcome, Settings, StateDir, TrustList, TrustedPeer,
    },
    unix_now,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
};

use crate::{
    discovery::{Advertisement, Discovered, Discovery, DiscoveryEvent, local_addrs},
    incoming, outgoing,
    session::{Control, Live, LiveCell, SessionEnd},
    tls::TlsContext,
};
use dexdrop_core::api::*;

/// How the engine is set up.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Private app data (identity, settings, journals).
    pub state_dir: PathBuf,
    /// Where received files go unless the user picks another folder.
    pub default_download_dir: PathBuf,
    /// `"linux"`, `"windows"`, `"macos"`, `"android"`, `"ios"`.
    pub platform: String,
    /// The device name until the user sets one.
    pub default_device_name: String,
    /// Advertise and browse with mDNS.
    pub discovery: bool,
    /// Discovery waits until the user has read why the local network is
    /// used (`Settings::local_network_explained`, FR-004), so a system
    /// permission prompt follows an explanation. Apps set it where the OS
    /// asks (iOS, macOS); default off.
    pub explain_local_network_first: bool,
    /// Transfers over the internet (Iroh, Phase 6); `None`: local only.
    pub remote: Option<crate::remote::RemoteConfig>,
    /// The address to listen on.
    pub listen: IpAddr,
    /// Overrides the settings' port.
    pub port: Option<u16>,
    pub chunk_size: u32,
    /// Caps the sending rate, in bytes per second (`None`: no cap).
    pub max_send_rate: Option<u64>,
    /// Caps the receiving rate, in bytes per second (`None`: no cap).
    pub max_receive_rate: Option<u64>,
    /// Where a new identity's private key is kept (and where an existing
    /// one moves to): the platform's choice unless set.
    pub key_storage: dexdrop_core::keystore::KeyStorage,
}

impl EngineConfig {
    pub fn new(
        state_dir: impl Into<PathBuf>,
        download_dir: impl Into<PathBuf>,
        platform: &str,
    ) -> Self {
        Self {
            state_dir: state_dir.into(),
            default_download_dir: download_dir.into(),
            platform: platform.to_owned(),
            default_device_name: String::new(),
            discovery: true,
            explain_local_network_first: false,
            remote: None,
            listen: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: None,
            chunk_size: DEFAULT_CHUNK_SIZE,
            max_send_rate: None,
            max_receive_rate: None,
            key_storage: dexdrop_core::keystore::KeyStorage::platform_default(),
        }
    }
}

/// Errors starting the engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("identity: {0}")]
    Identity(#[from] dexdrop_core::identity::IdentityError),
    #[error("TLS: {0}")]
    Tls(#[from] rustls::Error),
    #[error("network: {0}")]
    Io(#[from] std::io::Error),
}

/// A handle that sends commands to a running engine.
#[derive(Clone, Debug)]
pub struct EngineHandle {
    commands: mpsc::UnboundedSender<Command>,
}

impl EngineHandle {
    /// Sends a command; ignored once the engine stopped.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

/// An engine running on its own thread and Tokio runtime.
pub struct Engine {
    handle: EngineHandle,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    /// Starts the engine on a dedicated thread; events arrive on the receiver.
    pub fn start(
        config: EngineConfig,
    ) -> Result<(Engine, mpsc::UnboundedReceiver<Event>), EngineError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .thread_name("dexdrop-engine")
            .enable_all()
            .build()?;
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("dexdrop-engine".into())
            .spawn(move || {
                runtime.block_on(async move {
                    match spawn(config).await {
                        Ok((handle, events, done)) => {
                            let _ = ready_tx.send(Ok((handle, events)));
                            let _ = done.await;
                        }
                        Err(err) => {
                            let _ = ready_tx.send(Err(err));
                        }
                    }
                });
            })?;
        let (handle, events) = ready_rx
            .recv()
            .map_err(|_| std::io::Error::other("the engine thread stopped"))??;
        Ok((
            Engine {
                handle,
                thread: Some(thread),
            },
            events,
        ))
    }

    pub fn handle(&self) -> EngineHandle {
        self.handle.clone()
    }

    pub fn send(&self, command: Command) {
        self.handle.send(command);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.handle.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Starts the engine on the current Tokio runtime. The returned receiver
/// resolves when the engine stops.
pub async fn spawn(
    config: EngineConfig,
) -> Result<
    (
        EngineHandle,
        mpsc::UnboundedReceiver<Event>,
        oneshot::Receiver<()>,
    ),
    EngineError,
> {
    let state = StateDir::new(&config.state_dir);
    std::fs::create_dir_all(state.root())?;
    let identity = Identity::load_or_create_with(&state.identity(), config.key_storage)?;
    let tls = TlsContext::new(&identity)?;

    let mut settings = Settings::load(&state.settings());
    if settings.device_name.trim().is_empty() {
        settings.device_name = if config.default_device_name.trim().is_empty() {
            default_device_name(&config.platform, &identity.peer_id())
        } else {
            config.default_device_name.clone()
        };
        let _ = settings.save(&state.settings());
    }

    let port = config.port.unwrap_or(settings.port);
    let listener = match bind_listener(config.listen, port) {
        Ok(listener) => listener,
        Err(_) if port != 0 => bind_listener(config.listen, 0)?,
        Err(err) => return Err(err.into()),
    };
    let port = listener.local_addr()?.port();

    // The internet endpoint: a failure (no network, a blocked port) leaves
    // local transfers working.
    let remote = match &config.remote {
        Some(_) if !settings.internet => None,
        Some(remote_config) => match crate::remote::bind(&identity, remote_config).await {
            Ok(endpoint) => Some(endpoint),
            Err(err) => {
                tracing::warn!("internet transfers unavailable: {err}");
                None
            }
        },
        None => None,
    };

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (internal_tx, internal_rx) = mpsc::unbounded_channel();
    let (done_tx, done_rx) = oneshot::channel();

    let ctx = Arc::new(Ctx {
        identity,
        tls,
        state: state.clone(),
        platform: config.platform.clone(),
        chunk_size: config.chunk_size,
        max_send_rate: config.max_send_rate,
        max_receive_rate: config.max_receive_rate,
        settings: Mutex::new(settings.clone()),
        invitations: Mutex::new(InvitationBook::default()),
        peers: Mutex::new(HashMap::new()),
        internal: internal_tx.clone(),
        remote: remote.clone(),
        known: Mutex::new(dexdrop_core::store::KnownPeers::load(&state.known())),
    });

    // Incoming internet connections: the same handler as local ones, a
    // bounded number at once.
    let remote_task = remote.map(|endpoint| {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let slots = Arc::new(tokio::sync::Semaphore::new(MAX_INBOUND));
            while let Some(incoming) = endpoint.accept().await {
                let Ok(slot) = slots.clone().try_acquire_owned() else {
                    continue; // Dropped: too many connections.
                };
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    if let Ok(Ok(conn)) =
                        tokio::time::timeout(Duration::from_secs(20), incoming.into_future()).await
                        && let Ok(link) = crate::remote::accept(conn.clone()).await
                    {
                        incoming::handle(ctx, link, incoming::Via::Internet(conn)).await;
                    }
                    drop(slot);
                });
            }
        })
    });

    let accept_ctx = ctx.clone();
    let listener_task = tokio::spawn(async move {
        // Bounds what unknown peers can hold open: in total and per address.
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_INBOUND));
        let per_ip: Arc<Mutex<HashMap<IpAddr, usize>>> = Arc::default();
        loop {
            match listener.accept().await {
                Ok((tcp, from)) => {
                    // IPv4 peers reach the dual-stack socket as
                    // ::ffff:a.b.c.d: one count per peer either way.
                    let ip = from.ip().to_canonical();
                    let Ok(slot) = slots.clone().try_acquire_owned() else {
                        continue; // Dropped: too many connections.
                    };
                    {
                        let mut counts = lock(&per_ip);
                        let count = counts.entry(ip).or_default();
                        if *count >= MAX_INBOUND_PER_IP {
                            continue;
                        }
                        *count += 1;
                    }
                    let (ctx, per_ip) = (accept_ctx.clone(), per_ip.clone());
                    tokio::spawn(async move {
                        incoming::handle(ctx, crate::tls::tcp_link(tcp), incoming::Via::Nearby)
                            .await;
                        let mut counts = lock(&per_ip);
                        if let Some(count) = counts.get_mut(&ip) {
                            *count -= 1;
                            if *count == 0 {
                                counts.remove(&ip);
                            }
                        }
                        drop(slot);
                    });
                }
                Err(err) => {
                    tracing::warn!("accept failed: {err}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    });

    let mut actor = Actor {
        config,
        ctx,
        events: event_tx,
        trust: TrustList::load(&state.trusted()),
        history: History::load(&state.history()),
        transfers: HashMap::new(),
        order: Vec::new(),
        discovery: None,
        discovery_status: DiscoveryStatus::Disabled,
        advertised: None,
        addrs: Vec::new(),
        port,
        listener_task,
        remote_task,
        internal: None,
    };
    actor.boot(internal_tx);
    tokio::spawn(async move {
        actor.run(cmd_rx, internal_rx).await;
        let _ = done_tx.send(());
    });
    Ok((EngineHandle { commands: cmd_tx }, event_rx, done_rx))
}

/// Listens on `listen` and `port`. The default, the unspecified IPv4
/// address, means every address: one dual-stack socket takes IPv4 and IPv6,
/// because discovery advertises IPv6 addresses too and a peer may resolve
/// only those (Android's NsdManager often does). IPv4 only where IPv6 is
/// unavailable.
fn bind_listener(listen: IpAddr, port: u16) -> std::io::Result<TcpListener> {
    if listen == IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        && let Ok(listener) = bind_dual_stack(port)
    {
        return Ok(listener);
    }
    let listener = std::net::TcpListener::bind(SocketAddr::new(listen, port))?;
    listener.set_nonblocking(true)?;
    TcpListener::from_std(listener)
}

fn bind_dual_stack(port: u16) -> std::io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    // Windows defaults to IPv6 only; say so explicitly everywhere.
    socket.set_only_v6(false)?;
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port).into())?;
    socket.listen(1024)?;
    socket.set_nonblocking(true)?;
    TcpListener::from_std(socket.into())
}

fn default_device_name(platform: &str, id: &PeerId) -> String {
    let kind = match platform {
        "android" => "Android device",
        "ios" => "iPhone",
        "macos" => "Mac",
        "windows" => "Windows PC",
        "linux" => "Linux PC",
        "web" => "Browser",
        _ => "Device",
    };
    format!("{kind} {}", id.to_string()[..4].to_ascii_uppercase())
}

/// What the engine's tasks share.
pub(crate) struct Ctx {
    pub identity: Identity,
    pub tls: TlsContext,
    pub state: StateDir,
    pub platform: String,
    pub chunk_size: u32,
    pub max_send_rate: Option<u64>,
    pub max_receive_rate: Option<u64>,
    pub settings: Mutex<Settings>,
    pub invitations: Mutex<InvitationBook>,
    pub peers: Mutex<HashMap<PeerId, Discovered>>,
    pub internal: mpsc::UnboundedSender<Internal>,
    /// The internet endpoint, when transfers over the internet are on.
    pub remote: Option<iroh::Endpoint>,
    /// Devices this one exchanged files with: reachable over the internet.
    pub known: Mutex<dexdrop_core::store::KnownPeers>,
}

impl Ctx {
    pub fn device_name(&self) -> String {
        lock(&self.settings).device_name.clone()
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The decision about an incoming offer.
pub(crate) enum Decision {
    Accept {
        journal: Box<ReceiveJournal>,
        live: LiveCell,
        control: watch::Receiver<Control>,
    },
    Reject(RejectReason),
}

/// An incoming offer, after the handshake.
pub(crate) struct OfferArrived {
    pub peer_id: PeerId,
    pub peer_name: String,
    pub peer_platform: String,
    pub sas: String,
    pub via_invitation: bool,
    pub offer: crate::session::ReceivedOffer,
    pub decide: oneshot::Sender<Decision>,
    pub route: Route,
    /// The sender's raw Ed25519 key, remembered once the transfer completes.
    pub key: Option<[u8; 32]>,
}

/// Messages from the engine's tasks to the actor.
pub(crate) enum Internal {
    Discovery(DiscoveryEvent),
    Offer(Box<OfferArrived>),
    /// The sender of a pending request went away.
    RequestGone(TransferId),
    /// A code or invitation was used to connect.
    PairingUsed {
        code: bool,
        by: String,
    },
    /// An incoming session ended.
    IncomingEnded {
        id: TransferId,
        result: Result<SessionEnd, String>,
        journal: Box<ReceiveJournal>,
    },
    /// An outgoing transfer's manifest is ready (or failed).
    ManifestReady {
        id: TransferId,
        result: Result<Manifest, String>,
    },
    OutgoingState {
        id: TransferId,
        state: TransferState,
    },
    OutgoingPeer {
        id: TransferId,
        peer_id: PeerId,
        name: String,
        platform: String,
        sas: String,
        route: Route,
        key: Option<[u8; 32]>,
    },
    /// How a transfer's connection goes now.
    Route {
        id: TransferId,
        route: Route,
    },
    OutgoingEnded {
        id: TransferId,
        state: TransferState,
    },
}

struct Transfer {
    snap: TransferSnapshot,
    live: LiveCell,
    control: watch::Sender<Control>,
    decide: Option<oneshot::Sender<Decision>>,
    request: Option<(IncomingRequest, crate::session::ReceivedOffer)>,
    outgoing: Option<OutgoingPlan>,
    running: bool,
    rate: RateMeter,
    /// The other device's raw Ed25519 key, once connected.
    peer_key: Option<[u8; 32]>,
}

struct OutgoingPlan {
    target: Target,
    manifest: Option<Arc<Manifest>>,
    resume: bool,
}

#[derive(Default)]
struct RateMeter {
    last: Option<(Instant, u64)>,
    rate: Option<f64>,
    since: Option<Instant>,
    /// Time spent transferring and bytes moved in it, over all sessions:
    /// the measured average (FR-016), excluding waits and pauses.
    tick: Option<(Instant, u64)>,
    active: Duration,
    moved: u64,
}

impl RateMeter {
    /// Adds the time and bytes since the last tick while transferring.
    fn accumulate(&mut self, now: Instant, bytes: u64, active: bool) {
        if let Some((at, before)) = self.tick {
            self.active += now.duration_since(at);
            self.moved += bytes.saturating_sub(before);
        }
        self.tick = active.then_some((now, bytes));
    }

    /// The average rate, once enough was measured to mean something.
    fn average(&mut self, final_bytes: u64) -> Option<f64> {
        self.accumulate(Instant::now(), final_bytes, false);
        (self.active >= Duration::from_millis(800) && self.moved >= 1_000_000)
            .then(|| self.moved as f64 / self.active.as_secs_f64())
    }

    fn sample(&mut self, bytes: u64, active: bool) -> (Option<f64>, bool) {
        let now = Instant::now();
        self.accumulate(now, bytes, active);
        if !active {
            self.last = None;
            self.since = None;
            self.rate = None;
            return (None, false);
        }
        let since = *self.since.get_or_insert(now);
        if let Some((at, before)) = self.last {
            let dt = now.duration_since(at).as_secs_f64();
            if dt >= 0.2 {
                let instant = bytes.saturating_sub(before) as f64 / dt;
                self.rate = Some(match self.rate {
                    Some(r) => r * 0.7 + instant * 0.3,
                    None => instant,
                });
                self.last = Some((now, bytes));
            }
        } else {
            self.last = Some((now, bytes));
        }
        // An estimate only after a few seconds of samples (PRD 7.4).
        (
            self.rate,
            now.duration_since(since) >= Duration::from_secs(3),
        )
    }
}

struct Actor {
    config: EngineConfig,
    ctx: Arc<Ctx>,
    events: mpsc::UnboundedSender<Event>,
    trust: TrustList,
    history: History,
    transfers: HashMap<TransferId, Transfer>,
    order: Vec<TransferId>,
    discovery: Option<Discovery>,
    discovery_status: DiscoveryStatus,
    advertised: Option<Advertisement>,
    addrs: Vec<IpAddr>,
    port: u16,
    listener_task: tokio::task::JoinHandle<()>,
    remote_task: Option<tokio::task::JoinHandle<()>>,
    internal: Option<mpsc::UnboundedSender<Internal>>,
}

const MAX_PENDING_REQUESTS: usize = 8;
/// Inbound connections at once, in total and from one address.
const MAX_INBOUND: usize = 32;
const MAX_INBOUND_PER_IP: usize = 6;
/// Saved files listed on a finished incoming transfer (a share sheet takes
/// at most 100).
const MAX_SAVED_LISTED: usize = 100;
const STALE_JOURNAL_SECS: u64 = 7 * 24 * 3600;

impl Actor {
    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn notice(&self, kind: NoticeKind, text: impl Into<String>) {
        self.emit(Event::Notice {
            kind,
            text: text.into(),
        });
    }

    fn boot(&mut self, internal: mpsc::UnboundedSender<Internal>) {
        self.addrs = local_addrs();
        self.internal = Some(internal);
        if self.config.discovery && !self.awaiting_explanation() {
            self.start_discovery();
        }
        self.readvertise();
        self.restore();
        self.emit_status();
        self.emit(Event::Settings(lock(&self.ctx.settings).clone()));
        self.emit_peers();
        self.emit(Event::Trusted(self.trust.peers.clone()));
        self.emit(Event::History(self.history.entries.clone()));
    }

    /// Whether discovery waits for the user to read why the local network is
    /// used: on the platforms that ask permission for it (FR-004).
    fn awaiting_explanation(&self) -> bool {
        self.config.explain_local_network_first && !lock(&self.ctx.settings).local_network_explained
    }

    fn start_discovery(&mut self) {
        let Some(internal) = self.internal.clone() else {
            return;
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        match Discovery::start(self.ctx.identity.peer_id(), tx) {
            Ok(discovery) => {
                self.discovery = Some(discovery);
                self.discovery_status = DiscoveryStatus::Active;
                tokio::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        if internal.send(Internal::Discovery(event)).is_err() {
                            break;
                        }
                    }
                });
            }
            Err(err) => {
                self.discovery_status = DiscoveryStatus::Unavailable(err);
            }
        }
    }

    /// Loads interrupted transfers from disk (resume after restart).
    fn restore(&mut self) {
        let now = unix_now();
        for journal in self.ctx.state.incoming().list_journals() {
            if now.saturating_sub(journal.created) > STALE_JOURNAL_SECS {
                remove_partials(&journal);
                let _ = self.ctx.state.incoming().remove(&journal.transfer_id);
                continue;
            }
            let mut snap = blank_snapshot(journal.transfer_id, Direction::Incoming);
            snap.peer_id = Some(journal.peer_id);
            snap.peer_name = journal.peer_name.clone();
            snap.label = journal.label.clone();
            snap.total_bytes = journal.total_bytes;
            snap.bytes_done = journal.verified_bytes();
            snap.saved_to = Some(journal.destination.clone());
            snap.state = TransferState::Interrupted {
                reason: format!("Waiting for {} to resume.", journal.peer_name),
                retrying: false,
            };
            self.insert(snap, None);
        }
        for record in self.ctx.state.outgoing().list::<OutgoingRecord>() {
            let mut snap = blank_snapshot(record.transfer_id, Direction::Outgoing);
            snap.peer_id = Some(record.peer_id);
            snap.peer_name = record.peer_name.clone();
            snap.peer_platform = record.peer_platform.clone();
            fill_from_manifest(&mut snap, &record.manifest);
            snap.state = TransferState::Interrupted {
                reason: "Interrupted when the app closed.".into(),
                retrying: false,
            };
            let plan = OutgoingPlan {
                target: Target::Peer(record.peer_id),
                manifest: Some(Arc::new(record.manifest.clone())),
                resume: true,
            };
            self.insert(snap, Some(plan));
            if let Some(t) = self.transfers.get_mut(&record.transfer_id) {
                t.outgoing.as_mut().expect("just set").target = if record.addrs.is_empty() {
                    Target::Peer(record.peer_id)
                } else {
                    Target::Address {
                        addr: record.addrs[0],
                        peer_id: Some(record.peer_id),
                    }
                };
            }
        }
    }

    fn insert(&mut self, snap: TransferSnapshot, outgoing: Option<OutgoingPlan>) {
        let id = snap.id;
        let (control, _) = watch::channel(Control::Run);
        self.emit(Event::Transfer(snap.clone()));
        if !self.transfers.contains_key(&id) {
            self.order.push(id);
        }
        self.transfers.insert(
            id,
            Transfer {
                snap,
                live: Arc::new(Mutex::new(Live::default())),
                control,
                decide: None,
                request: None,
                outgoing,
                running: false,
                rate: RateMeter::default(),
                peer_key: None,
            },
        );
    }

    async fn run(
        &mut self,
        mut commands: mpsc::UnboundedReceiver<Command>,
        mut internal: mpsc::UnboundedReceiver<Internal>,
    ) {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut slow_tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Shutdown) | None => break,
                    Some(command) => self.command(command),
                },
                Some(message) = internal.recv() => self.internal(message),
                _ = tick.tick() => self.sample(),
                _ = slow_tick.tick() => self.housekeeping(),
            }
        }
        self.shutdown();
        // Iroh says goodbye to peers and relays before the runtime stops.
        if let Some(endpoint) = self.ctx.remote.clone() {
            let _ = tokio::time::timeout(Duration::from_secs(2), endpoint.close()).await;
        }
    }

    fn shutdown(&mut self) {
        for t in self.transfers.values() {
            if t.running {
                let _ = t.control.send(Control::Pause);
            }
        }
        self.listener_task.abort();
        if let Some(task) = &self.remote_task {
            task.abort();
        }
        self.discovery = None;
        self.emit(Event::Stopped);
    }

    // ------------------------------------------------------------ commands

    fn command(&mut self, command: Command) {
        match command {
            Command::SendFiles { target, paths } => self.send_files(target, paths),
            Command::Respond {
                id,
                accept,
                trust,
                conflict,
            } => self.respond(id, accept, trust, conflict),
            Command::Pause(id) => {
                if let Some(t) = self.transfers.get_mut(&id)
                    && t.snap.can_pause()
                {
                    let _ = t.control.send(Control::Pause);
                    if !t.running {
                        t.snap.state = TransferState::Paused { by_peer: false };
                        let _ = self.events.send(Event::Transfer(t.snap.clone()));
                    }
                }
            }
            Command::Resume(id) => self.resume(id),
            Command::Cancel(id) => self.cancel(id),
            Command::Dismiss(id) => {
                if self
                    .transfers
                    .get(&id)
                    .is_some_and(|t| t.snap.can_dismiss())
                {
                    self.transfers.remove(&id);
                    self.order.retain(|x| x != &id);
                    self.emit(Event::TransferRemoved(id));
                }
            }
            Command::ClearFinished => {
                let finished: Vec<_> = self
                    .transfers
                    .iter()
                    .filter(|(_, t)| t.snap.can_dismiss())
                    .map(|(id, _)| *id)
                    .collect();
                for id in finished {
                    self.transfers.remove(&id);
                    self.order.retain(|x| x != &id);
                    self.emit(Event::TransferRemoved(id));
                }
            }
            Command::CreateInvitation => self.create_invitation(),
            Command::CreateCode => {
                let (code, expires) = lock(&self.ctx.invitations).create_code(DEFAULT_TTL_SECS);
                self.readvertise();
                self.emit(Event::Code { code, expires });
            }
            Command::CloseInvitations => {
                lock(&self.ctx.invitations).clear();
                self.readvertise();
                self.emit(Event::InvitationsClosed);
            }
            Command::SetDeviceName(name) => {
                let name: String = name.trim().chars().take(48).collect();
                if name.is_empty() {
                    self.notice(NoticeKind::Error, "The device name cannot be empty.");
                    return;
                }
                self.update_settings(|s| s.device_name = name);
                self.readvertise();
                self.emit_status();
            }
            Command::SetVisible(visible) => {
                self.update_settings(|s| s.visible = visible);
                self.readvertise();
                self.emit_status();
            }
            Command::SetDownloadDir(dir) => self.update_settings(|s| s.download_dir = dir),
            Command::SetAutoAcceptTrusted(on) => {
                self.update_settings(|s| s.auto_accept_trusted = on)
            }
            Command::SetKeepHistory(on) => {
                self.update_settings(|s| s.keep_history = on);
                if !on {
                    self.clear_history();
                }
            }
            Command::SetAllowMetered(on) => self.update_settings(|s| s.allow_metered = on),
            Command::SetPeerAutoAccept {
                peer_id,
                auto_accept,
            } => {
                if let Some(peer) = self.trust.peers.iter_mut().find(|p| p.peer_id == peer_id) {
                    peer.auto_accept = auto_accept;
                    let _ = self.trust.save(&self.ctx.state.trusted());
                    self.emit(Event::Trusted(self.trust.peers.clone()));
                }
            }
            Command::ForgetPeer(peer_id) => {
                if self.trust.forget(&peer_id) {
                    let _ = self.trust.save(&self.ctx.state.trusted());
                    self.emit(Event::Trusted(self.trust.peers.clone()));
                }
                let forgot = lock(&self.ctx.known).forget(&peer_id);
                if forgot {
                    let _ = lock(&self.ctx.known).save(&self.ctx.state.known());
                }
                self.emit_peers();
            }
            Command::ClearHistory => self.clear_history(),
            Command::SetInternet(on) => self.update_settings(|s| s.internet = on),
            Command::LocalNetworkExplained => {
                self.update_settings(|s| s.local_network_explained = true);
                if self.config.discovery && self.discovery.is_none() {
                    self.start_discovery();
                    self.advertised = None;
                    self.readvertise();
                    self.emit_status();
                }
            }
            Command::RefreshDiscovery => {
                self.addrs = local_addrs();
                self.advertised = None;
                self.readvertise();
                self.emit_status();
            }
            Command::ExportDiagnostics => {
                let report = self.diagnostics();
                self.emit(Event::Diagnostics(report));
            }
            Command::Shutdown => {}
        }
    }

    fn update_settings(&mut self, f: impl FnOnce(&mut Settings)) {
        let settings = {
            let mut settings = lock(&self.ctx.settings);
            f(&mut settings);
            settings.clone()
        };
        if let Err(err) = settings.save(&self.ctx.state.settings()) {
            self.notice(NoticeKind::Error, format!("Could not save settings: {err}"));
        }
        self.emit(Event::Settings(settings));
    }

    fn clear_history(&mut self) {
        self.history.entries.clear();
        let _ = self.history.save(&self.ctx.state.history());
        self.emit(Event::History(Vec::new()));
    }

    fn send_files(&mut self, target: Target, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let id = dexdrop_core::random_id();
        let mut snap = blank_snapshot(id, Direction::Outgoing);
        match &target {
            Target::Peer(peer_id) => {
                snap.peer_id = Some(*peer_id);
                if let Some(peer) = lock(&self.ctx.peers).get(peer_id) {
                    snap.peer_name = peer.name.clone();
                    snap.peer_platform = peer.platform.clone();
                } else if let Some(trusted) = self.trust.get(peer_id) {
                    snap.peer_name = trusted.name.clone();
                    snap.peer_platform = trusted.platform.clone();
                }
            }
            Target::Invitation(text) => match invitation::Invitation::decode(text, unix_now()) {
                Ok(inv) => {
                    snap.peer_id = Some(inv.peer_id);
                    snap.peer_name = inv.name;
                }
                Err(err) => {
                    self.notice(NoticeKind::Error, err.to_string());
                    return;
                }
            },
            Target::Code(code) => {
                if invitation::normalize_code(code).is_none() {
                    self.notice(
                        NoticeKind::Error,
                        "Codes have 8 letters and digits, like K7P2-QX9M.",
                    );
                    return;
                }
                snap.peer_name = "Device with code".into();
            }
            Target::Address { addr, peer_id } => {
                snap.peer_id = *peer_id;
                snap.peer_name = addr.to_string();
            }
        }
        snap.state = TransferState::Preparing;
        snap.label = paths
            .first()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.insert(
            snap,
            Some(OutgoingPlan {
                target,
                manifest: None,
                resume: false,
            }),
        );
        let internal = self.ctx.internal.clone();
        tokio::task::spawn_blocking(move || {
            let result = Manifest::build(&paths).map_err(|e| e.to_string());
            let _ = internal.send(Internal::ManifestReady { id, result });
        });
    }

    fn start_outgoing(&mut self, id: TransferId) {
        let Some(t) = self.transfers.get_mut(&id) else {
            return;
        };
        let Some(plan) = &mut t.outgoing else { return };
        let Some(manifest) = plan.manifest.clone() else {
            return;
        };
        if t.running {
            return;
        }
        let (control, control_rx) = watch::channel(Control::Run);
        t.control = control;
        t.running = true;
        *lock(&t.live) = Live::default();
        t.snap.state = TransferState::Connecting;
        let job = outgoing::Job {
            id,
            target: plan.target.clone(),
            manifest,
            resume: plan.resume,
            live: t.live.clone(),
            control: control_rx,
        };
        plan.resume = true;
        let _ = self.events.send(Event::Transfer(t.snap.clone()));
        tokio::spawn(outgoing::run(self.ctx.clone(), job));
    }

    fn resume(&mut self, id: TransferId) {
        let Some(t) = self.transfers.get_mut(&id) else {
            return;
        };
        if !t.snap.can_resume() {
            return;
        }
        if t.running {
            let _ = t.control.send(Control::Run);
        } else {
            self.start_outgoing(id);
        }
    }

    fn cancel(&mut self, id: TransferId) {
        let Some(t) = self.transfers.get_mut(&id) else {
            return;
        };
        if !t.snap.can_cancel() {
            return;
        }
        if t.running {
            let _ = t.control.send(Control::Cancel);
            return;
        }
        // Not connected: clean up locally.
        match t.snap.direction {
            Direction::Incoming => {
                if let Ok(Some(journal)) = self.ctx.state.incoming().load_journal(&id) {
                    remove_partials(&journal);
                }
                let _ = self.ctx.state.incoming().remove(&id);
            }
            Direction::Outgoing => {
                let _ = self.ctx.state.outgoing().remove(&id);
            }
        }
        t.snap.state = TransferState::Cancelled { by_peer: false };
        t.snap.rate = None;
        t.snap.eta = None;
        let snap = t.snap.clone();
        self.emit(Event::Transfer(snap.clone()));
        self.record_history(&snap);
    }

    fn respond(&mut self, id: TransferId, accept: bool, trust: bool, conflict: ConflictPolicy) {
        let Some(t) = self.transfers.get_mut(&id) else {
            return;
        };
        let (Some(decide), Some((request, offer))) = (t.decide.take(), t.request.take()) else {
            return;
        };
        self.emit(Event::RequestClosed(id));
        if !accept {
            let _ = decide.send(Decision::Reject(RejectReason::Declined));
            let t = self.transfers.get_mut(&id).expect("present");
            t.snap.state = TransferState::Declined("You declined.".into());
            let snap = t.snap.clone();
            self.emit(Event::Transfer(snap.clone()));
            self.record_history(&snap);
            return;
        }
        if request
            .free_space
            .is_some_and(|free| free < request.total_bytes)
        {
            let _ = decide.send(Decision::Reject(RejectReason::NotEnoughSpace));
            let t = self.transfers.get_mut(&id).expect("present");
            t.snap.state = TransferState::Failed("Not enough free space.".into());
            let _ = self.events.send(Event::Transfer(t.snap.clone()));
            self.notice(
                NoticeKind::Error,
                "There is not enough free space for this transfer.",
            );
            return;
        }
        if trust {
            self.trust.trust(TrustedPeer {
                peer_id: request.peer_id,
                name: request.peer_name.clone(),
                platform: request.peer_platform.clone(),
                auto_accept: false,
                added: unix_now(),
            });
            let _ = self.trust.save(&self.ctx.state.trusted());
            self.emit(Event::Trusted(self.trust.peers.clone()));
            self.emit_peers();
        }
        let journal = new_journal(&request, &offer, conflict);
        self.start_incoming(id, journal, decide);
    }

    fn start_incoming(
        &mut self,
        id: TransferId,
        mut journal: ReceiveJournal,
        decide: oneshot::Sender<Decision>,
    ) {
        let Some(t) = self.transfers.get_mut(&id) else {
            return;
        };
        let (control, control_rx) = watch::channel(Control::Run);
        t.control = control;
        t.running = true;
        *lock(&t.live) = Live::default();
        t.snap.saved_to = Some(journal.destination.clone());
        t.snap.state = TransferState::Transferring;
        let _ = self.ctx.state.incoming().save_journal(&mut journal);
        let decision = Decision::Accept {
            journal: Box::new(journal),
            live: t.live.clone(),
            control: control_rx,
        };
        if decide.send(decision).is_err() {
            t.running = false;
            t.snap.state = TransferState::Interrupted {
                reason: "The other device left.".into(),
                retrying: false,
            };
        }
        let _ = self.events.send(Event::Transfer(t.snap.clone()));
    }

    fn create_invitation(&mut self) {
        let addrs: Vec<SocketAddr> = self
            .addrs
            .iter()
            .take(4)
            .map(|ip| SocketAddr::new(*ip, self.port))
            .collect();
        if addrs.is_empty() && self.ctx.remote.is_none() {
            self.notice(
                NoticeKind::Error,
                "This device is not connected to a network.",
            );
            return;
        }
        let name = self.ctx.device_name();
        let remote = self.ctx.remote.as_ref().and_then(|endpoint| {
            Some(dexdrop_core::invitation::RemoteHint {
                key: self.ctx.identity.ed25519_key()?,
                relay: crate::remote::home_relay(endpoint),
            })
        });
        let inv = lock(&self.ctx.invitations).create(
            self.ctx.identity.peer_id(),
            &name,
            addrs,
            remote,
            DEFAULT_TTL_SECS,
        );
        let text = inv.encode();
        let (qr_width, qr_modules) = invitation::qr_matrix(&text).unwrap_or((0, Vec::new()));
        self.emit(Event::Invitation(InvitationView {
            text,
            qr_width,
            qr_modules,
            expires: inv.expires,
        }));
    }

    // ------------------------------------------------------------ internal

    fn internal(&mut self, message: Internal) {
        match message {
            Internal::Discovery(event) => self.discovery_event(event),
            Internal::Offer(arrived) => self.offer_arrived(*arrived),
            Internal::PairingUsed { code, by } => {
                // The code's flag leaves the advertisement at once.
                self.readvertise();
                self.emit(Event::PairingUsed { code, by });
            }
            Internal::RequestGone(id) => {
                if let Some(t) = self.transfers.get_mut(&id)
                    && t.snap.state == TransferState::Requested
                {
                    // Never answered: nothing to keep, nothing for history
                    // (a device sending and withdrawing offers in a loop
                    // must not grow either).
                    self.transfers.remove(&id);
                    self.order.retain(|x| x != &id);
                    self.emit(Event::RequestClosed(id));
                    self.emit(Event::TransferRemoved(id));
                }
            }
            Internal::IncomingEnded {
                id,
                result,
                journal,
            } => self.incoming_ended(id, result, *journal),
            Internal::ManifestReady { id, result } => match result {
                Ok(manifest) => {
                    let Some(t) = self.transfers.get_mut(&id) else {
                        return;
                    };
                    fill_from_manifest(&mut t.snap, &manifest);
                    if manifest.skipped > 0 {
                        self.notice(
                            NoticeKind::Info,
                            format!("{} links or special files were left out.", manifest.skipped),
                        );
                    }
                    let t = self.transfers.get_mut(&id).expect("present");
                    if let Some(plan) = &mut t.outgoing {
                        plan.manifest = Some(Arc::new(manifest));
                    }
                    if t.snap.state == TransferState::Preparing {
                        self.start_outgoing(id);
                    }
                }
                Err(err) => {
                    if let Some(t) = self.transfers.get_mut(&id) {
                        t.snap.state =
                            TransferState::Failed(format!("Could not read the files: {err}"));
                        let _ = self.events.send(Event::Transfer(t.snap.clone()));
                    }
                }
            },
            Internal::OutgoingState { id, state } => {
                if let Some(t) = self.transfers.get_mut(&id) {
                    t.snap.state = state;
                    if t.snap.state != TransferState::Transferring {
                        t.snap.rate = None;
                        t.snap.eta = None;
                    }
                    let _ = self.events.send(Event::Transfer(t.snap.clone()));
                }
            }
            Internal::OutgoingPeer {
                id,
                peer_id,
                name,
                platform,
                sas,
                route,
                key,
            } => {
                if let Some(t) = self.transfers.get_mut(&id) {
                    t.snap.peer_id = Some(peer_id);
                    t.snap.peer_name = name.clone();
                    t.snap.peer_platform = platform.clone();
                    t.snap.sas = Some(sas);
                    t.snap.route = Some(route);
                    t.peer_key = key;
                    let _ = self.events.send(Event::Transfer(t.snap.clone()));
                }
                // A device this one reached: reachable over the internet later.
                if let Some(key) = key {
                    self.remember(peer_id, name, platform, key);
                }
            }
            Internal::Route { id, route } => {
                if let Some(t) = self.transfers.get_mut(&id)
                    && t.snap.route != Some(route)
                {
                    t.snap.route = Some(route);
                    let _ = self.events.send(Event::Transfer(t.snap.clone()));
                }
            }
            Internal::OutgoingEnded { id, state } => {
                let Some(t) = self.transfers.get_mut(&id) else {
                    return;
                };
                t.running = false;
                apply_live(&mut t.snap, &t.live);
                if state.is_finished() {
                    t.snap.average_rate = t.rate.average(t.snap.bytes_done);
                }
                t.snap.state = state;
                t.snap.rate = None;
                t.snap.eta = None;
                t.snap.current = None;
                if t.snap.state == TransferState::Completed {
                    t.snap.bytes_done = t.snap.total_bytes;
                }
                let snap = t.snap.clone();
                self.emit(Event::Transfer(snap.clone()));
                if snap.state.is_finished() {
                    self.record_history(&snap);
                    if snap.state == TransferState::Completed {
                        self.notice(
                            NoticeKind::Success,
                            format!("Sent {} to {}.", snap.label, snap.peer_name),
                        );
                    }
                }
            }
        }
    }

    fn discovery_event(&mut self, event: DiscoveryEvent) {
        match event {
            DiscoveryEvent::Found(mut found) => {
                // Addresses are added to, not replaced: an announcement
                // claiming a device's ID with another address cannot make
                // the real one unreachable (the TLS identity is checked at
                // each address anyway).
                let mut peers = lock(&self.ctx.peers);
                if let Some(known) = peers.get(&found.peer_id) {
                    for addr in &known.addrs {
                        if !found.addrs.contains(addr) && found.addrs.len() < 8 {
                            found.addrs.push(*addr);
                        }
                    }
                }
                peers.insert(found.peer_id, found);
                drop(peers);
                self.emit_peers();
            }
            DiscoveryEvent::Lost(id) => {
                lock(&self.ctx.peers).remove(&id);
                self.emit_peers();
            }
            DiscoveryEvent::Unavailable(reason) => {
                self.discovery_status = DiscoveryStatus::Unavailable(reason);
                self.emit_status();
            }
        }
    }

    fn offer_arrived(&mut self, arrived: OfferArrived) {
        let OfferArrived {
            peer_id,
            peer_name,
            peer_platform,
            sas,
            via_invitation,
            offer,
            decide,
            route,
            key,
        } = arrived;
        let id = offer.offer.transfer_id;

        // A transfer this device accepted before: resume without asking.
        if let Ok(Some(journal)) = self.ctx.state.incoming().load_journal(&id) {
            let digest = dexdrop_core::protocol::manifest_digest(
                &id,
                offer.offer.chunk_size,
                &offer.entries,
            );
            if journal.peer_id != peer_id || journal.manifest_digest != digest {
                let _ = decide.send(Decision::Reject(RejectReason::InvalidManifest));
                return;
            }
            if self.transfers.get(&id).is_some_and(|t| t.running) {
                let _ = decide.send(Decision::Reject(RejectReason::Busy));
                return;
            }
            let mut snap = self
                .transfers
                .get(&id)
                .map(|t| t.snap.clone())
                .unwrap_or_else(|| blank_snapshot(id, Direction::Incoming));
            snap.peer_id = Some(peer_id);
            snap.peer_name = peer_name;
            snap.peer_platform = peer_platform;
            snap.sas = Some(sas);
            snap.route = Some(route);
            fill_from_offer(&mut snap, &offer);
            self.insert(snap, None);
            if let Some(t) = self.transfers.get_mut(&id) {
                t.peer_key = key;
            }
            self.start_incoming(id, journal, decide);
            return;
        }
        if self.transfers.contains_key(&id) {
            let _ = decide.send(Decision::Reject(RejectReason::Busy));
            return;
        }
        // One unanswered request per device, a few in total: a device that
        // floods offers cannot crowd out the others.
        let pending = || {
            self.transfers
                .values()
                .filter(|t| t.snap.state == TransferState::Requested)
        };
        if pending().count() >= MAX_PENDING_REQUESTS
            || pending().any(|t| t.snap.peer_id == Some(peer_id))
        {
            let _ = decide.send(Decision::Reject(RejectReason::Busy));
            return;
        }

        let settings = lock(&self.ctx.settings).clone();
        let destination = settings
            .download_dir
            .clone()
            .unwrap_or_else(|| self.config.default_download_dir.clone());
        let _ = std::fs::create_dir_all(&destination);
        let trusted = self.trust.get(&peer_id).cloned();
        let request = IncomingRequest {
            id,
            peer_id,
            peer_name: peer_name.clone(),
            peer_platform: peer_platform.clone(),
            fingerprint: peer_id.fingerprint(),
            sas: sas.clone(),
            label: offer.offer.label.clone(),
            files: offer.offer.files,
            folders: offer.offer.folders,
            total_bytes: offer.offer.total_bytes,
            sample: offer
                .components
                .iter()
                .filter(|c| c.len() == 1)
                .take(5)
                .map(|c| c[0].clone())
                .collect(),
            trusted: trusted.is_some(),
            via_invitation,
            destination,
            free_space: dexdrop_core::fsutil::available_space(
                settings
                    .download_dir
                    .as_deref()
                    .unwrap_or(&self.config.default_download_dir),
            ),
            route,
        };
        let mut snap = blank_snapshot(id, Direction::Incoming);
        snap.peer_id = Some(peer_id);
        snap.peer_name = peer_name;
        snap.peer_platform = peer_platform;
        snap.sas = Some(sas);
        snap.saved_to = Some(request.destination.clone());
        snap.route = Some(route);
        fill_from_offer(&mut snap, &offer);
        snap.state = TransferState::Requested;
        self.insert(snap, None);
        if let Some(t) = self.transfers.get_mut(&id) {
            t.peer_key = key;
        }

        let auto = settings.auto_accept_trusted
            && trusted.is_some_and(|p| p.auto_accept)
            && request.enough_space();
        if auto {
            let journal = new_journal(&request, &offer, ConflictPolicy::KeepBoth);
            self.start_incoming(id, journal, decide);
            self.notice(
                NoticeKind::Info,
                format!("Receiving {} from {}.", request.label, request.peer_name),
            );
            return;
        }
        let t = self.transfers.get_mut(&id).expect("just inserted");
        t.decide = Some(decide);
        t.request = Some((request.clone(), offer));
        self.emit(Event::Request(request));
    }

    fn incoming_ended(
        &mut self,
        id: TransferId,
        result: Result<SessionEnd, String>,
        journal: ReceiveJournal,
    ) {
        let mut remember_after = Vec::new();
        let Some(t) = self.transfers.get_mut(&id) else {
            return;
        };
        t.running = false;
        apply_live(&mut t.snap, &t.live);
        if matches!(result, Ok(SessionEnd::Completed)) {
            t.snap.average_rate = t.rate.average(t.snap.bytes_done);
        }
        t.snap.rate = None;
        t.snap.eta = None;
        t.snap.current = None;
        let records = self.ctx.state.incoming();
        t.snap.state = match result {
            Ok(SessionEnd::Completed) => {
                let _ = records.remove(&id);
                t.snap.bytes_done = t.snap.total_bytes;
                if let (Some(peer), Some(key)) = (t.snap.peer_id, t.peer_key) {
                    remember_after.push((
                        peer,
                        t.snap.peer_name.clone(),
                        t.snap.peer_platform.clone(),
                        key,
                    ));
                }
                // What was saved, in manifest order, for sharing/exporting.
                t.snap.saved_files = journal
                    .files
                    .values()
                    .filter_map(|f| f.saved.clone())
                    .take(MAX_SAVED_LISTED)
                    .collect();
                if t.snap.files_failed > 0 || t.snap.files_skipped > 0 {
                    TransferState::CompletedWithIssues
                } else {
                    TransferState::Completed
                }
            }
            Ok(SessionEnd::Paused { by_peer }) => TransferState::Paused { by_peer },
            Ok(SessionEnd::OutOfSpace { .. }) => TransferState::Interrupted {
                reason: format!(
                    "This device ran out of storage space. Free some space, then ask {} to resume.",
                    t.snap.peer_name
                ),
                retrying: false,
            },
            Ok(SessionEnd::Cancelled { by_peer }) => {
                remove_partials(&journal);
                let _ = records.remove(&id);
                TransferState::Cancelled { by_peer }
            }
            Ok(SessionEnd::Rejected(reason)) => TransferState::Declined(reason.explain().into()),
            Err(reason) => TransferState::Interrupted {
                reason: format!("{reason}. Waiting for {} to resume.", t.snap.peer_name),
                retrying: false,
            },
        };
        let snap = t.snap.clone();
        self.emit(Event::Transfer(snap.clone()));
        if snap.state.is_finished() {
            self.record_history(&snap);
            if snap.state == TransferState::Completed {
                self.notice(
                    NoticeKind::Success,
                    format!("Received {} from {}.", snap.label, snap.peer_name),
                );
            }
        }
        for (peer, name, platform, key) in remember_after {
            self.remember(peer, name, platform, key);
        }
    }

    // ------------------------------------------------------------ periodic

    fn sample(&mut self) {
        for id in &self.order {
            let Some(t) = self.transfers.get_mut(id) else {
                continue;
            };
            if !t.running {
                continue;
            }
            let before = t.snap.clone();
            apply_live(&mut t.snap, &t.live);
            let active = t.snap.state == TransferState::Transferring;
            let (rate, stable) = t.rate.sample(t.snap.bytes_done, active);
            t.snap.rate = rate;
            t.snap.eta = match rate {
                Some(r) if stable && r > 1.0 => Some(
                    (t.snap.total_bytes.saturating_sub(t.snap.bytes_done) as f64 / r).ceil() as u64,
                ),
                _ => None,
            };
            if t.snap != before {
                let _ = self.events.send(Event::Transfer(t.snap.clone()));
            }
        }
    }

    fn housekeeping(&mut self) {
        // Network changes (FR-013) and code expiry change the advertisement.
        let addrs = local_addrs();
        if addrs != self.addrs {
            self.addrs = addrs;
            self.advertised = None;
            self.emit_status();
        }
        self.readvertise();
    }

    fn readvertise(&mut self) {
        let settings = lock(&self.ctx.settings).clone();
        let wanted = settings.visible.then(|| Advertisement {
            peer_id: self.ctx.identity.peer_id(),
            name: settings.device_name.clone(),
            platform: self.ctx.platform.clone(),
            port: self.port,
            code_open: lock(&self.ctx.invitations).has_open_code(unix_now()),
        });
        if wanted == self.advertised {
            return;
        }
        let Some(discovery) = &mut self.discovery else {
            self.advertised = wanted;
            return;
        };
        match &wanted {
            Some(ad) => {
                if let Err(err) = discovery.advertise(ad) {
                    self.discovery_status = DiscoveryStatus::Unavailable(err);
                }
            }
            None => discovery.withdraw(),
        }
        self.advertised = wanted;
    }

    fn emit_status(&self) {
        let settings = lock(&self.ctx.settings).clone();
        self.emit(Event::Status(Status {
            peer_id: self.ctx.identity.peer_id(),
            fingerprint: self.ctx.identity.peer_id().fingerprint(),
            device_name: settings.device_name,
            platform: self.ctx.platform.clone(),
            port: self.port,
            addrs: self.addrs.clone(),
            visible: settings.visible,
            discovery: self.discovery_status.clone(),
            internet: self.ctx.remote.is_some(),
        }));
    }

    fn last_rate(&self, peer: &PeerId) -> Option<f64> {
        self.history
            .entries
            .iter()
            .find(|e| e.peer_id == Some(*peer) && e.average_rate.is_some())
            .and_then(|e| e.average_rate)
    }

    /// Remembers a device this one exchanged files with, with its key.
    fn remember(&mut self, peer_id: PeerId, name: String, platform: String, key: [u8; 32]) {
        let peer = dexdrop_core::store::KnownPeer {
            peer_id,
            name,
            platform,
            key,
            relay: None,
            last_seen: unix_now(),
        };
        let mut known = lock(&self.ctx.known);
        if known.remember(peer) {
            let _ = known.save(&self.ctx.state.known());
        }
        drop(known);
        self.emit_peers();
    }

    fn emit_peers(&self) {
        let mut peers: Vec<PeerInfo> = lock(&self.ctx.peers)
            .values()
            .map(|d| PeerInfo {
                peer_id: d.peer_id,
                name: d.name.clone(),
                platform: d.platform.clone(),
                fingerprint: d.peer_id.fingerprint(),
                trusted: self.trust.get(&d.peer_id).is_some(),
                code_open: d.code_open,
                last_rate: self.last_rate(&d.peer_id),
                nearby: true,
            })
            .collect();
        // Known devices not on this network: over the internet.
        if self.ctx.remote.is_some() {
            for k in &lock(&self.ctx.known).peers {
                if peers.iter().any(|p| p.peer_id == k.peer_id) {
                    continue;
                }
                peers.push(PeerInfo {
                    peer_id: k.peer_id,
                    name: k.name.clone(),
                    platform: k.platform.clone(),
                    fingerprint: k.peer_id.fingerprint(),
                    trusted: self.trust.get(&k.peer_id).is_some(),
                    code_open: false,
                    last_rate: self.last_rate(&k.peer_id),
                    nearby: false,
                });
            }
        }
        peers.sort_by(|a, b| {
            b.trusted
                .cmp(&a.trusted)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.emit(Event::Peers(peers));
    }

    fn record_history(&mut self, snap: &TransferSnapshot) {
        if !lock(&self.ctx.settings).keep_history {
            return;
        }
        let outcome = match &snap.state {
            TransferState::Completed => Outcome::Completed,
            TransferState::CompletedWithIssues => Outcome::Partial {
                failed: snap.files_failed,
                skipped: snap.files_skipped,
            },
            TransferState::Declined(_) => Outcome::Declined,
            TransferState::Cancelled { .. } => Outcome::Cancelled,
            TransferState::Failed(reason) => Outcome::Failed(reason.clone()),
            _ => return,
        };
        self.history.push(HistoryEntry {
            transfer_id: dexdrop_core::hex(&snap.id),
            direction: snap.direction,
            peer_name: snap.peer_name.clone(),
            peer_platform: snap.peer_platform.clone(),
            label: snap.label.clone(),
            files: snap.files,
            bytes: snap.total_bytes,
            outcome,
            finished: unix_now(),
            saved_to: snap.saved_to.clone(),
            peer_id: snap.peer_id,
            average_rate: snap.average_rate,
        });
        let _ = self.history.save(&self.ctx.state.history());
        self.emit(Event::History(self.history.entries.clone()));
        // A new measurement may change what nearby devices show.
        self.emit_peers();
    }

    /// An anonymized report: no file names, paths, keys or tokens (FR-075).
    fn diagnostics(&self) -> String {
        let mut by_state: HashMap<String, u64> = HashMap::new();
        for t in self.transfers.values() {
            let key = match &t.snap.state {
                TransferState::Interrupted { .. } => "interrupted".to_owned(),
                TransferState::Declined(_) => "declined".to_owned(),
                TransferState::Failed(_) => "failed".to_owned(),
                other => format!("{other:?}").to_lowercase(),
            };
            *by_state.entry(key).or_default() += 1;
        }
        let outcomes: Vec<String> = self
            .history
            .entries
            .iter()
            .take(50)
            .map(|e| {
                let outcome = match &e.outcome {
                    Outcome::Completed => "completed",
                    Outcome::Partial { .. } => "partial",
                    Outcome::Declined => "declined",
                    Outcome::Cancelled => "cancelled",
                    Outcome::Failed(_) => "failed",
                };
                format!(
                    "{{\"direction\":\"{:?}\",\"peer_platform\":\"{}\",\"files\":{},\"bytes\":{},\"outcome\":\"{}\"}}",
                    e.direction, e.peer_platform, e.files, e.bytes, outcome
                )
            })
            .collect();
        let discovery = match &self.discovery_status {
            DiscoveryStatus::Active => "active",
            DiscoveryStatus::Disabled => "disabled",
            DiscoveryStatus::Unavailable(_) => "unavailable",
        };
        format!(
            "{{\n  \"app_version\": \"{}\",\n  \"protocol\": {},\n  \"os\": \"{}\",\n  \"arch\": \"{}\",\n  \"platform\": \"{}\",\n  \"transport\": \"tcp+tls1.3\",\n  \"identity_key\": \"{}\",\n  \"discovery\": \"{}\",\n  \"interfaces\": {},\n  \"nearby_devices\": {},\n  \"transfers_by_state\": {:?},\n  \"recent\": [{}]\n}}\n",
            env!("CARGO_PKG_VERSION"),
            dexdrop_core::protocol::PROTOCOL_VERSION,
            std::env::consts::OS,
            std::env::consts::ARCH,
            self.ctx.platform,
            self.ctx.identity.key_storage().describe(),
            discovery,
            self.addrs.len(),
            lock(&self.ctx.peers).len(),
            by_state,
            outcomes.join(","),
        )
    }
}

fn blank_snapshot(id: TransferId, direction: Direction) -> TransferSnapshot {
    TransferSnapshot {
        id,
        direction,
        peer_id: None,
        peer_name: String::new(),
        peer_platform: String::new(),
        label: String::new(),
        files: 0,
        folders: 0,
        total_bytes: 0,
        bytes_done: 0,
        files_done: 0,
        files_failed: 0,
        files_skipped: 0,
        rate: None,
        eta: None,
        state: TransferState::Preparing,
        sas: None,
        saved_to: None,
        failures: Vec::new(),
        current: None,
        started: unix_now(),
        average_rate: None,
        saved_files: Vec::new(),
        route: None,
    }
}

fn fill_from_manifest(snap: &mut TransferSnapshot, manifest: &Manifest) {
    snap.label = manifest.label.clone();
    snap.files = manifest.files;
    snap.folders = manifest.folders;
    snap.total_bytes = manifest.total_bytes;
}

fn fill_from_offer(snap: &mut TransferSnapshot, offer: &crate::session::ReceivedOffer) {
    snap.label = offer.offer.label.clone();
    snap.files = offer.offer.files;
    snap.folders = offer.offer.folders;
    snap.total_bytes = offer.offer.total_bytes;
}

fn apply_live(snap: &mut TransferSnapshot, live: &LiveCell) {
    let live = lock(live);
    snap.bytes_done = live.bytes_done.min(snap.total_bytes);
    snap.files_done = live.files_done;
    snap.files_failed = live.files_failed;
    snap.files_skipped = live.files_skipped;
    snap.current = live.current.clone();
    snap.failures = live
        .failures
        .iter()
        .map(|(path, failure)| (path.clone(), failure.explain().to_owned()))
        .collect();
}

fn new_journal(
    request: &IncomingRequest,
    offer: &crate::session::ReceivedOffer,
    conflict: ConflictPolicy,
) -> ReceiveJournal {
    let renames = if conflict == ConflictPolicy::KeepBoth {
        crate::session::plan_renames(offer, &request.destination)
    } else {
        Default::default()
    };
    ReceiveJournal {
        version: 1,
        transfer_id: request.id,
        peer_id: request.peer_id,
        peer_name: request.peer_name.clone(),
        manifest_digest: dexdrop_core::protocol::manifest_digest(
            &request.id,
            offer.offer.chunk_size,
            &offer.entries,
        ),
        destination: request.destination.clone(),
        conflict,
        label: request.label.clone(),
        total_bytes: request.total_bytes,
        created: unix_now(),
        renames,
        files: Default::default(),
        log_generation: 0,
    }
}

/// A user-facing size summary: `"3 files, 1 folder · 12.3 MB"`.
pub fn summary(files: u64, folders: u64, bytes: u64) -> String {
    format!(
        "{} · {}",
        format::item_count(files, folders),
        format::bytes(bytes)
    )
}
