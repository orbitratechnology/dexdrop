//! One transfer session over an established, authenticated stream.
//!
//! [`run_sender`] and [`run_receiver`] implement steps 4–6 of the protocol
//! (see [`dexdrop_core::protocol`]): bounded, pipelined file requests, chunked
//! streaming with per-chunk and whole-file BLAKE3 verification, temporary
//! files with atomic commit, and a journal that lets the next session resume
//! from the last verified chunk.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use dexdrop_core::{
    journal::{FileProgress, FileState, ReceiveJournal, RecordDir, partial_path},
    manifest::{Manifest, mtime_of},
    naming::{self, ConflictPolicy},
    pathsafe,
    protocol::{
        self, CancelReason, ChunkHeader, EntryKind, FileFailure, MANIFEST_PAGE_ENTRIES,
        MAX_CHUNK_SIZE, MAX_ENTRIES, MIN_CHUNK_SIZE, ManifestPage, Message, Offer, RejectReason,
        WireEntry,
    },
};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{Semaphore, mpsc, watch},
};

use crate::wire::{self, WireError};

/// What the user asked a running session to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Run,
    Pause,
    Cancel,
}

/// Live counters of a session, read by the engine a few times a second.
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub bytes_done: u64,
    pub files_done: u64,
    pub files_failed: u64,
    pub files_skipped: u64,
    pub current: Option<String>,
    /// The first few failures, for the user.
    pub failures: Vec<(String, FileFailure)>,
}

impl Live {
    fn fail(&mut self, path: &str, failure: FileFailure) {
        self.files_failed += 1;
        if self.failures.len() < 20 {
            self.failures.push((path.to_owned(), failure));
        }
    }
}

/// Shared live counters.
pub type LiveCell = Arc<Mutex<Live>>;

/// How a session ended without an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    Completed,
    Paused { by_peer: bool },
    Cancelled { by_peer: bool },
    Rejected(RejectReason),
}

/// Why a session failed.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("the other device broke the protocol: {0}")]
    Protocol(String),
    #[error("the other device is not the one expected")]
    WrongPeer,
    #[error("the destination ran out of space")]
    NoSpace,
    #[error("could not write the destination: {0}")]
    Disk(std::io::Error),
}

impl SessionError {
    /// Whether retrying (reconnecting and resuming) can help.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            SessionError::Wire(WireError::Io(_) | WireError::Closed | WireError::Timeout)
        )
    }
}

fn protocol_err(what: impl Into<String>) -> SessionError {
    SessionError::Protocol(what.into())
}

fn lock(live: &LiveCell) -> std::sync::MutexGuard<'_, Live> {
    live.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------- offers

/// Sends the offer and the manifest, in pages.
pub async fn send_offer<W: AsyncWrite + Unpin>(
    w: &mut W,
    offer: &Offer,
    entries: &[WireEntry],
) -> Result<(), SessionError> {
    wire::write_msg(w, &Message::Offer(offer.clone())).await?;
    let mut pages = entries.chunks(MANIFEST_PAGE_ENTRIES).peekable();
    if pages.peek().is_none() {
        wire::write_msg(
            w,
            &Message::ManifestPage(ManifestPage {
                entries: Vec::new(),
                last: true,
            }),
        )
        .await?;
    }
    while let Some(page) = pages.next() {
        let last = pages.peek().is_none();
        wire::write_msg(
            w,
            &Message::ManifestPage(ManifestPage {
                entries: page.to_vec(),
                last,
            }),
        )
        .await?;
    }
    w.flush().await.map_err(WireError::from)?;
    Ok(())
}

/// A validated offer: its manifest and every path's safe components.
#[derive(Clone, Debug)]
pub struct ReceivedOffer {
    pub offer: Offer,
    pub entries: Arc<Vec<WireEntry>>,
    pub components: Arc<Vec<Vec<String>>>,
}

/// Reads and validates an offer and its manifest (FR-045, FR-057).
pub async fn read_offer<R: AsyncRead + Unpin>(
    r: &mut R,
) -> Result<Result<ReceivedOffer, RejectReason>, SessionError> {
    let limit = Duration::from_secs(30);
    let offer = match wire::read_msg_within(r, limit).await? {
        Message::Offer(offer) => offer,
        Message::Cancel(_) => return Err(SessionError::Wire(WireError::Closed)),
        other => {
            return Err(protocol_err(format!(
                "expected Offer, got {}",
                other.name()
            )));
        }
    };
    if offer.entries > MAX_ENTRIES || !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&offer.chunk_size)
    {
        return Ok(Err(RejectReason::InvalidManifest));
    }
    let mut entries = Vec::with_capacity(offer.entries.min(4096) as usize);
    loop {
        match wire::read_msg_within(r, limit).await? {
            Message::ManifestPage(page) => {
                if entries.len() as u64 + page.entries.len() as u64 > offer.entries {
                    return Ok(Err(RejectReason::InvalidManifest));
                }
                entries.extend(page.entries);
                if page.last {
                    break;
                }
            }
            other => {
                return Err(protocol_err(format!(
                    "expected ManifestPage, got {}",
                    other.name()
                )));
            }
        }
    }
    Ok(validate(offer, entries))
}

fn validate(offer: Offer, entries: Vec<WireEntry>) -> Result<ReceivedOffer, RejectReason> {
    if entries.len() as u64 != offer.entries || entries.is_empty() {
        return Err(RejectReason::InvalidManifest);
    }
    let mut files = 0u64;
    let mut folders = 0u64;
    let mut bytes = 0u64;
    let mut components = Vec::with_capacity(entries.len());
    for entry in &entries {
        let comps = pathsafe::sanitize(&entry.path).map_err(|_| RejectReason::InvalidManifest)?;
        match entry.kind {
            EntryKind::File => {
                files += 1;
                bytes = bytes
                    .checked_add(entry.size)
                    .ok_or(RejectReason::InvalidManifest)?;
            }
            EntryKind::Folder => {
                if entry.size != 0 {
                    return Err(RejectReason::InvalidManifest);
                }
                folders += 1;
            }
        }
        components.push(comps);
    }
    if files != offer.files || folders != offer.folders || bytes != offer.total_bytes {
        return Err(RejectReason::InvalidManifest);
    }
    Ok(ReceivedOffer {
        offer,
        entries: Arc::new(entries),
        components: Arc::new(components),
    })
}

/// The receiver's answer as the sender sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Accepted,
    Rejected(RejectReason),
    Cancelled,
}

/// Waits for the receiver to accept, while the user may still cancel.
pub async fn await_answer<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    control: &mut watch::Receiver<Control>,
) -> Result<Answer, SessionError> {
    let limit = Duration::from_secs(10 * 60);
    loop {
        tokio::select! {
            changed = control.changed() => {
                if changed.is_err() || *control.borrow() == Control::Cancel {
                    let _ = wire::send(stream, &Message::Cancel(CancelReason::User)).await;
                    return Ok(Answer::Cancelled);
                }
                // A pause while waiting changes nothing.
            }
            msg = wire::read_msg_within(stream, limit) => {
                return match msg? {
                    Message::Accept => Ok(Answer::Accepted),
                    Message::Reject(reason) => Ok(Answer::Rejected(reason)),
                    Message::Cancel(_) => Ok(Answer::Rejected(RejectReason::Declined)),
                    other => Err(protocol_err(format!("expected an answer, got {}", other.name()))),
                };
            }
        }
    }
}

// ---------------------------------------------------------------- sender

/// Streams the requested files until the receiver completes, or the user or
/// the peer pauses or cancels.
pub async fn run_sender<S>(
    stream: S,
    manifest: Arc<Manifest>,
    chunk_size: u32,
    live: LiveCell,
    mut control: watch::Receiver<Control>,
) -> Result<SessionEnd, SessionError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (rd, mut wr) = tokio::io::split(stream);
    let (tx, mut rx) = mpsc::unbounded_channel::<Result<Message, WireError>>();
    let peer_stopped = Arc::new(Stop::default());
    let reader = {
        let peer_stopped = peer_stopped.clone();
        tokio::spawn(async move {
            let mut rd = BufReader::new(rd);
            loop {
                match wire::read_msg(&mut rd).await {
                    Ok(msg) => {
                        let stop =
                            matches!(msg, Message::Complete | Message::Pause | Message::Cancel(_));
                        if stop {
                            peer_stopped.set();
                        }
                        if tx.send(Ok(msg)).is_err() || stop {
                            break;
                        }
                    }
                    Err(err) => {
                        peer_stopped.set();
                        let _ = tx.send(Err(err));
                        break;
                    }
                }
            }
        })
    };
    let _reader = AbortOnDrop(reader);

    let mut buf = Vec::new();
    let mut counted: HashMap<u32, u64> = HashMap::new();
    loop {
        let next = tokio::select! {
            biased;
            changed = control.changed() => {
                let wanted = if changed.is_err() { Control::Cancel } else { *control.borrow() };
                match wanted {
                    Control::Run => continue,
                    Control::Pause => {
                        let _ = wire::send(&mut wr, &Message::Pause).await;
                        let _ = wr.shutdown().await;
                        return Ok(SessionEnd::Paused { by_peer: false });
                    }
                    Control::Cancel => {
                        let _ = wire::send(&mut wr, &Message::Cancel(CancelReason::User)).await;
                        let _ = wr.shutdown().await;
                        return Ok(SessionEnd::Cancelled { by_peer: false });
                    }
                }
            }
            next = rx.recv() => next,
        };
        let msg = match next {
            Some(Ok(msg)) => msg,
            Some(Err(err)) => return Err(err.into()),
            None => return Err(WireError::Closed.into()),
        };
        match msg {
            Message::Want { index, offset } => {
                let entry = manifest
                    .entries
                    .get(index as usize)
                    .ok_or_else(|| protocol_err("request for an unknown file"))?;
                // A file requested again (verification retry) restarts its count.
                if let Some(previous) = counted.remove(&index) {
                    let mut l = lock(&live);
                    l.bytes_done = l.bytes_done.saturating_sub(previous);
                }
                match stream_file(
                    &mut wr,
                    entry,
                    index,
                    offset,
                    chunk_size,
                    &mut buf,
                    &live,
                    &control,
                    &peer_stopped,
                )
                .await
                {
                    Ok(sent) => {
                        counted.insert(index, sent);
                    }
                    Err(err) => {
                        // A peer that paused or cancelled stops reading; its
                        // message explains the failed write.
                        while let Ok(Ok(msg)) = rx.try_recv() {
                            match msg {
                                Message::Pause => return Ok(SessionEnd::Paused { by_peer: true }),
                                Message::Cancel(_) => {
                                    return Ok(SessionEnd::Cancelled { by_peer: true });
                                }
                                _ => {}
                            }
                        }
                        return Err(err);
                    }
                }
            }
            Message::Skip {
                index,
                already_received,
            } => {
                let entry = manifest
                    .entries
                    .get(index as usize)
                    .ok_or_else(|| protocol_err("skip of an unknown file"))?;
                let mut l = lock(&live);
                l.bytes_done += entry.wire.size;
                if already_received {
                    l.files_done += 1;
                } else {
                    l.files_skipped += 1;
                }
            }
            Message::FileResult { index, failure } => {
                let path = manifest
                    .entries
                    .get(index as usize)
                    .map(|e| e.wire.path.clone())
                    .unwrap_or_default();
                let mut l = lock(&live);
                match failure {
                    None => l.files_done += 1,
                    Some(failure) => l.fail(&path, failure),
                }
            }
            Message::Complete => {
                let _ = wr.shutdown().await;
                return Ok(SessionEnd::Completed);
            }
            Message::Pause => return Ok(SessionEnd::Paused { by_peer: true }),
            Message::Cancel(_) => return Ok(SessionEnd::Cancelled { by_peer: true }),
            other => return Err(protocol_err(format!("unexpected {}", other.name()))),
        }
    }
}

/// Sends one file from `offset`; returns the bytes counted as done.
#[allow(clippy::too_many_arguments)]
async fn stream_file<W: AsyncWrite + Unpin>(
    wr: &mut W,
    entry: &dexdrop_core::manifest::SourceEntry,
    index: u32,
    offset: u64,
    chunk_size: u32,
    buf: &mut Vec<u8>,
    live: &LiveCell,
    control: &watch::Receiver<Control>,
    peer_stopped: &Stop,
) -> Result<u64, SessionError> {
    let (Some(source), EntryKind::File) = (&entry.source, entry.wire.kind) else {
        return Err(protocol_err("request for a folder"));
    };
    let size = entry.wire.size;
    let chunk = u64::from(chunk_size);
    if offset > size || (!offset.is_multiple_of(chunk) && offset != size) {
        return Err(protocol_err("request from an invalid offset"));
    }
    let fail = |failure| Message::FileError { index, failure };

    let mut file = match File::open(source).await {
        Ok(file) => file,
        Err(_) => {
            wire::send(wr, &fail(FileFailure::SourceChanged)).await?;
            return Ok(0);
        }
    };
    if !unchanged(&file, entry).await {
        wire::send(wr, &fail(FileFailure::SourceChanged)).await?;
        return Ok(0);
    }
    if buf.len() < chunk_size as usize {
        buf.resize(chunk_size as usize, 0);
    }

    // The whole-file hash covers the bytes the receiver already has, too.
    let mut whole = blake3::Hasher::new();
    let mut pos = 0u64;
    while pos < offset {
        let n = chunk.min(offset - pos) as usize;
        if file.read_exact(&mut buf[..n]).await.is_err() {
            wire::send(wr, &fail(FileFailure::SourceChanged)).await?;
            return Ok(0);
        }
        whole.update(&buf[..n]);
        pos += n as u64;
    }

    wire::write_msg(wr, &Message::FileStart { index, offset }).await?;
    {
        let mut l = lock(live);
        l.current = Some(entry.wire.path.clone());
        l.bytes_done += offset;
    }
    let mut counted = offset;
    let mut watch_control = control.clone();
    while pos < size {
        if peer_stopped.is_set() || *control.borrow() != Control::Run {
            // The main loop sends Pause/Cancel or reads the peer's.
            return Ok(counted);
        }
        let n = chunk.min(size - pos) as usize;
        if file.read_exact(&mut buf[..n]).await.is_err() {
            wire::send(wr, &fail(FileFailure::SourceChanged)).await?;
            return Ok(counted);
        }
        let data = &buf[..n];
        whole.update(data);
        let header = ChunkHeader {
            index,
            offset: pos,
            len: n as u32,
            hash: protocol::chunk_hash(data),
        };
        // A write blocks while the receiver does not read: give up when it
        // stops, the user pauses, or nothing moves for a minute.
        let write = async {
            wire::write_msg(wr, &Message::Chunk(header)).await?;
            wr.write_all(data).await.map_err(WireError::from)
        };
        tokio::select! {
            written = write => written?,
            _ = peer_stopped.wait() => return Ok(counted),
            _ = watch_control.changed() => return Ok(counted),
            _ = tokio::time::sleep(WRITE_STALL) => return Err(WireError::Timeout.into()),
        }
        pos += n as u64;
        counted += n as u64;
        lock(live).bytes_done += n as u64;
    }
    if !unchanged(&file, entry).await {
        wire::send(wr, &fail(FileFailure::SourceChanged)).await?;
        return Ok(counted);
    }
    wire::send(
        wr,
        &Message::FileEnd {
            index,
            hash: *whole.finalize().as_bytes(),
        },
    )
    .await?;
    Ok(counted)
}

/// How long a chunk write may block before the session is considered lost.
const WRITE_STALL: Duration = Duration::from_secs(60);

/// Set once the peer paused, cancelled, completed or went away.
#[derive(Default)]
struct Stop {
    flag: AtomicBool,
    notify: tokio::sync::Notify,
}

impl Stop {
    fn set(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }

    fn is_set(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    async fn wait(&self) {
        if !self.is_set() {
            self.notify.notified().await;
        }
    }
}

async fn unchanged(file: &File, entry: &dexdrop_core::manifest::SourceEntry) -> bool {
    match file.metadata().await {
        Ok(meta) => meta.len() == entry.wire.size && mtime_of(&meta) == entry.wire.mtime,
        Err(_) => false,
    }
}

// ---------------------------------------------------------------- receiver

/// The most file requests outstanding at once.
const MAX_OUTSTANDING: usize = 64;
/// How often verified progress is flushed and journaled.
const JOURNAL_EVERY: Duration = Duration::from_secs(2);

/// Everything a receiving session needs.
pub struct ReceivePlan {
    pub offer: ReceivedOffer,
    pub journal: ReceiveJournal,
    pub records: RecordDir,
}

#[derive(Clone, Debug)]
struct Planned {
    planned: PathBuf,
    temp: PathBuf,
    offset: u64,
    retried: bool,
}

struct Shared {
    requested: Mutex<HashMap<u32, Planned>>,
    resolved: AtomicU64,
    total_files: u64,
    completed: AtomicBool,
    failed: Mutex<Option<String>>,
    permits: Semaphore,
    writer: mpsc::UnboundedSender<Message>,
}

impl Shared {
    fn resolve_one(&self) {
        let n = self.resolved.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.total_files {
            self.complete();
        }
    }

    fn complete(&self) {
        if !self.completed.swap(true, Ordering::SeqCst) {
            let _ = self.writer.send(Message::Complete);
        }
    }

    fn requested(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Planned>> {
        self.requested.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Maps the first component through the journal's top-level renames.
fn mapped(
    components: &[String],
    renames: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let mut out = components.to_vec();
    if let Some(first) = out.first_mut()
        && let Some(renamed) = renames.get(first)
    {
        *first = renamed.clone();
    }
    out
}

/// Chooses "keep both" names for top-level folders that already exist.
pub fn plan_renames(
    offer: &ReceivedOffer,
    destination: &Path,
) -> std::collections::BTreeMap<String, String> {
    let mut renames = std::collections::BTreeMap::new();
    for (entry, comps) in offer.entries.iter().zip(offer.components.iter()) {
        if entry.kind == EntryKind::Folder
            && comps.len() == 1
            && destination.join(&comps[0]).exists()
        {
            let free = naming::first_free(destination, &comps[0]);
            if let Some(name) = free.file_name() {
                renames.insert(comps[0].clone(), name.to_string_lossy().into_owned());
            }
        }
    }
    renames
}

/// Bytes still needed, for the free-space check (FR-044).
pub fn bytes_needed(offer: &ReceivedOffer, journal: &ReceiveJournal) -> u64 {
    offer
        .offer
        .total_bytes
        .saturating_sub(journal.verified_bytes())
}

/// Receives the files of an accepted offer.
///
/// Returns how the session ended and the journal as it stands; the caller
/// removes the journal of a finished transfer.
pub async fn run_receiver<S>(
    stream: S,
    plan: ReceivePlan,
    live: LiveCell,
    mut control: watch::Receiver<Control>,
) -> (Result<SessionEnd, SessionError>, ReceiveJournal)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let ReceivePlan {
        offer,
        mut journal,
        records,
    } = plan;
    let (rd, mut wr) = tokio::io::split(stream);
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel::<Message>();
    let writer = tokio::spawn(async move {
        while let Some(msg) = writer_rx.recv().await {
            if wire::write_msg(&mut wr, &msg).await.is_err() {
                return;
            }
            if writer_rx.is_empty() && wr.flush().await.is_err() {
                return;
            }
        }
        let _ = wr.flush().await;
        let _ = wr.shutdown().await;
    });

    let total_files = offer
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::File)
        .count() as u64;
    let shared = Arc::new(Shared {
        requested: Mutex::new(HashMap::new()),
        resolved: AtomicU64::new(0),
        total_files,
        completed: AtomicBool::new(false),
        failed: Mutex::new(None),
        permits: Semaphore::new(MAX_OUTSTANDING),
        writer: writer_tx,
    });

    {
        let mut l = lock(&live);
        l.bytes_done = 0;
    }
    let planner = tokio::spawn(plan_requests(
        offer.clone(),
        journal.clone(),
        shared.clone(),
        live.clone(),
    ));
    let _planner = AbortOnDrop(planner);

    let mut receiver = Receiver {
        offer: &offer,
        journal: &mut journal,
        records: &records,
        shared: &shared,
        live: &live,
        current: None,
        buf: Vec::new(),
        last_save: Instant::now(),
    };
    let result = receiver.read_loop(rd, &mut control).await;
    let _ = records.save(&journal.transfer_id, &journal);
    // Let final messages (Complete, Pause, Cancel) reach the sender.
    drop(shared);
    drop(_planner);
    let mut writer = writer;
    if tokio::time::timeout(Duration::from_secs(5), &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
    (result, journal)
}

async fn plan_requests(
    offer: ReceivedOffer,
    journal: ReceiveJournal,
    shared: Arc<Shared>,
    live: LiveCell,
) {
    let destination = journal.destination.clone();
    for (i, (entry, comps)) in offer
        .entries
        .iter()
        .zip(offer.components.iter())
        .enumerate()
    {
        let index = i as u32;
        let comps = mapped(comps, &journal.renames);
        let target = match pathsafe::resolve(&destination, &comps) {
            Ok(path) => path,
            Err(err) => {
                // The destination holds a symbolic link where this goes.
                *shared.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(err.to_string());
                let _ = shared.writer.send(Message::Cancel(CancelReason::Error));
                return;
            }
        };
        match entry.kind {
            EntryKind::Folder => {
                if let Err(err) = tokio::fs::create_dir_all(&target).await {
                    *shared.failed.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(err.to_string());
                    let _ = shared.writer.send(Message::Cancel(CancelReason::Error));
                    return;
                }
            }
            EntryKind::File => {
                let progress = journal.files.get(&index).cloned().unwrap_or_default();
                match progress.state {
                    FileState::Done => {
                        let _ = shared.writer.send(Message::Skip {
                            index,
                            already_received: true,
                        });
                        let mut l = lock(&live);
                        l.bytes_done += entry.size;
                        l.files_done += 1;
                        drop(l);
                        shared.resolve_one();
                        continue;
                    }
                    FileState::Skipped => {
                        let _ = shared.writer.send(Message::Skip {
                            index,
                            already_received: false,
                        });
                        let mut l = lock(&live);
                        l.bytes_done += entry.size;
                        l.files_skipped += 1;
                        drop(l);
                        shared.resolve_one();
                        continue;
                    }
                    _ => {}
                }
                if journal.conflict == ConflictPolicy::Skip
                    && tokio::fs::symlink_metadata(&target).await.is_ok()
                {
                    let _ = shared.writer.send(Message::Skip {
                        index,
                        already_received: false,
                    });
                    let mut l = lock(&live);
                    l.bytes_done += entry.size;
                    l.files_skipped += 1;
                    drop(l);
                    shared.resolve_one();
                    continue;
                }
                let temp = progress
                    .temp
                    .clone()
                    .unwrap_or_else(|| partial_path(&target, &journal.transfer_id, index));
                let mut offset = 0;
                if progress.state == FileState::Partial
                    && let Ok(meta) = tokio::fs::metadata(&temp).await
                    && meta.len() >= progress.verified
                {
                    offset = progress.verified;
                }
                let Ok(permit) = shared.permits.acquire().await else {
                    return;
                };
                permit.forget();
                shared.requested().insert(
                    index,
                    Planned {
                        planned: target,
                        temp,
                        offset,
                        retried: false,
                    },
                );
                if shared.writer.send(Message::Want { index, offset }).is_err() {
                    return;
                }
            }
        }
    }
    if shared.total_files == 0 {
        shared.complete();
    }
}

struct Current {
    index: u32,
    file: File,
    offset: u64,
    size: u64,
    whole: blake3::Hasher,
    corrupt: bool,
    counted: u64,
}

struct Receiver<'a> {
    offer: &'a ReceivedOffer,
    journal: &'a mut ReceiveJournal,
    records: &'a RecordDir,
    shared: &'a Arc<Shared>,
    live: &'a LiveCell,
    current: Option<Current>,
    buf: Vec<u8>,
    last_save: Instant,
}

fn is_no_space(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::StorageFull
        || matches!(err.raw_os_error(), Some(28) | Some(112) | Some(39))
}

impl Receiver<'_> {
    async fn read_loop<R: AsyncRead + Unpin>(
        &mut self,
        rd: R,
        control: &mut watch::Receiver<Control>,
    ) -> Result<SessionEnd, SessionError> {
        let mut rd = BufReader::with_capacity(256 * 1024, rd);
        loop {
            let limit = if self.shared.completed.load(Ordering::SeqCst) {
                Duration::from_secs(5)
            } else {
                Duration::from_secs(60)
            };
            let msg = tokio::select! {
                biased;
                changed = control.changed() => {
                    let wanted = if changed.is_err() { Control::Cancel } else { *control.borrow() };
                    match wanted {
                        Control::Run => continue,
                        Control::Pause => {
                            self.save_progress().await;
                            let _ = self.shared.writer.send(Message::Pause);
                            return Ok(SessionEnd::Paused { by_peer: false });
                        }
                        Control::Cancel => {
                            let _ = self.shared.writer.send(Message::Cancel(CancelReason::User));
                            return Ok(SessionEnd::Cancelled { by_peer: false });
                        }
                    }
                }
                msg = wire::read_msg_within(&mut rd, limit) => msg,
            };
            let msg = match msg {
                Ok(msg) => msg,
                Err(err) => {
                    if self.shared.completed.load(Ordering::SeqCst) {
                        return Ok(SessionEnd::Completed);
                    }
                    self.save_progress().await;
                    if let Some(failed) = self
                        .shared
                        .failed
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take()
                    {
                        return Err(SessionError::Disk(std::io::Error::other(failed)));
                    }
                    return Err(err.into());
                }
            };
            match msg {
                Message::FileStart { index, offset } => self.start_file(index, offset).await?,
                Message::Chunk(header) => self.chunk(&mut rd, header).await?,
                Message::FileEnd { index, hash } => self.end_file(index, hash).await?,
                Message::FileError { index, failure } => self.file_error(index, failure).await,
                Message::Pause => {
                    self.save_progress().await;
                    return Ok(SessionEnd::Paused { by_peer: true });
                }
                Message::Cancel(_) => return Ok(SessionEnd::Cancelled { by_peer: true }),
                other => return Err(protocol_err(format!("unexpected {}", other.name()))),
            }
        }
    }

    fn entry(&self, index: u32) -> Result<&WireEntry, SessionError> {
        self.offer
            .entries
            .get(index as usize)
            .filter(|e| e.kind == EntryKind::File)
            .ok_or_else(|| protocol_err("data for an unknown file"))
    }

    async fn start_file(&mut self, index: u32, offset: u64) -> Result<(), SessionError> {
        if self.current.is_some() {
            return Err(protocol_err("a file started before the last one ended"));
        }
        let size = self.entry(index)?.size;
        let planned = self
            .shared
            .requested()
            .get(&index)
            .cloned()
            .ok_or_else(|| protocol_err("data for a file not requested"))?;
        if planned.offset != offset {
            return Err(protocol_err("data from an offset not requested"));
        }
        if let Some(parent) = planned.temp.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(SessionError::Disk)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&planned.temp)
            .await
            .map_err(SessionError::Disk)?;
        // Drop anything past the verified prefix, then hash the prefix.
        file.set_len(offset).await.map_err(SessionError::Disk)?;
        let mut whole = blake3::Hasher::new();
        if offset > 0 {
            file.seek(std::io::SeekFrom::Start(0))
                .await
                .map_err(SessionError::Disk)?;
            let mut prefix = (&mut file).take(offset);
            let mut tmp = vec![0u8; 1024 * 1024];
            loop {
                let n = prefix.read(&mut tmp).await.map_err(SessionError::Disk)?;
                if n == 0 {
                    break;
                }
                whole.update(&tmp[..n]);
            }
        }
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(SessionError::Disk)?;

        self.journal.files.insert(
            index,
            FileProgress {
                verified: offset,
                state: FileState::Partial,
                temp: Some(planned.temp.clone()),
                saved: None,
            },
        );
        {
            let mut l = lock(self.live);
            l.current = Some(self.offer.entries[index as usize].path.clone());
            l.bytes_done += offset;
        }
        self.current = Some(Current {
            index,
            file,
            offset,
            size,
            whole,
            corrupt: false,
            counted: offset,
        });
        Ok(())
    }

    async fn chunk<R: AsyncRead + Unpin>(
        &mut self,
        rd: &mut R,
        header: ChunkHeader,
    ) -> Result<(), SessionError> {
        let chunk_size = self.offer.offer.chunk_size;
        let Some(cur) = self.current.as_mut() else {
            return Err(protocol_err("a chunk outside a file"));
        };
        if header.index != cur.index
            || header.offset != cur.offset
            || header.len == 0
            || header.len > chunk_size
            || cur.offset + u64::from(header.len) > cur.size
        {
            return Err(protocol_err("a chunk out of place"));
        }
        let len = header.len as usize;
        if self.buf.len() < len {
            self.buf.resize(chunk_size as usize, 0);
        }
        rd.read_exact(&mut self.buf[..len])
            .await
            .map_err(WireError::from)?;
        let data = &self.buf[..len];
        if cur.corrupt || protocol::chunk_hash(data) != header.hash {
            // Keep reading this file's chunks, but write nothing more.
            cur.corrupt = true;
            return Ok(());
        }
        if let Err(err) = cur.file.write_all(data).await {
            return Err(if is_no_space(&err) {
                SessionError::NoSpace
            } else {
                SessionError::Disk(err)
            });
        }
        cur.whole.update(data);
        cur.offset += len as u64;
        cur.counted += len as u64;
        lock(self.live).bytes_done += len as u64;
        if self.last_save.elapsed() >= JOURNAL_EVERY {
            self.save_progress().await;
        }
        Ok(())
    }

    /// Flushes the current file and journals its verified prefix.
    async fn save_progress(&mut self) {
        if let Some(cur) = self.current.as_mut() {
            let durable = cur.file.flush().await.is_ok() && cur.file.sync_data().await.is_ok();
            if durable
                && !cur.corrupt
                && let Some(progress) = self.journal.files.get_mut(&cur.index)
            {
                // Only whole chunks count: resume restarts at a chunk boundary.
                let chunk = u64::from(self.offer.offer.chunk_size);
                progress.verified = if cur.offset == cur.size {
                    cur.offset
                } else {
                    cur.offset / chunk * chunk
                };
            }
        }
        let _ = self.records.save(&self.journal.transfer_id, &*self.journal);
        self.last_save = Instant::now();
    }

    async fn end_file(&mut self, index: u32, hash: [u8; 32]) -> Result<(), SessionError> {
        let Some(mut cur) = self.current.take() else {
            return Err(protocol_err("a file ended that did not start"));
        };
        if cur.index != index {
            return Err(protocol_err("the wrong file ended"));
        }
        let verified =
            !cur.corrupt && cur.offset == cur.size && *cur.whole.finalize().as_bytes() == hash;
        let planned = self.shared.requested().get(&index).cloned();
        let Some(planned) = planned else {
            return Err(protocol_err("a file ended that was not requested"));
        };
        if !verified {
            drop(cur.file);
            {
                let mut l = lock(self.live);
                l.bytes_done = l.bytes_done.saturating_sub(cur.counted);
            }
            if !planned.retried {
                // Ask for the whole file once more (FR-056).
                let _ = tokio::fs::remove_file(&planned.temp).await;
                self.shared.requested().insert(
                    index,
                    Planned {
                        offset: 0,
                        retried: true,
                        ..planned
                    },
                );
                if let Some(progress) = self.journal.files.get_mut(&index) {
                    progress.verified = 0;
                }
                let _ = self.shared.writer.send(Message::Want { index, offset: 0 });
                return Ok(());
            }
            let _ = tokio::fs::remove_file(&planned.temp).await;
            self.finish(index, Err(FileFailure::IntegrityMismatch), cur.counted);
            return Ok(());
        }
        if let Err(err) = async {
            cur.file.flush().await?;
            cur.file.sync_all().await
        }
        .await
        {
            return Err(if is_no_space(&err) {
                SessionError::NoSpace
            } else {
                SessionError::Disk(err)
            });
        }
        drop(cur.file);
        match commit(&planned.temp, &planned.planned, self.journal.conflict).await {
            Ok(saved) => {
                self.finish(index, Ok(saved), 0);
            }
            Err(_) => {
                let _ = tokio::fs::remove_file(&planned.temp).await;
                self.finish(index, Err(FileFailure::WriteFailed), cur.counted);
            }
        }
        if self.last_save.elapsed() >= JOURNAL_EVERY {
            self.save_progress().await;
        }
        Ok(())
    }

    async fn file_error(&mut self, index: u32, failure: FileFailure) {
        let counted = match self.current.take() {
            Some(cur) if cur.index == index => cur.counted,
            Some(cur) => {
                self.current = Some(cur);
                0
            }
            None => 0,
        };
        let planned = self.shared.requested().get(&index).cloned();
        if let Some(planned) = planned {
            let _ = tokio::fs::remove_file(&planned.temp).await;
        }
        {
            let mut l = lock(self.live);
            l.bytes_done = l.bytes_done.saturating_sub(counted);
        }
        // The sender knows; the file is resolved without a FileResult.
        self.resolve(index, Err(failure), false);
    }

    fn finish(&mut self, index: u32, result: Result<PathBuf, FileFailure>, uncount: u64) {
        {
            let mut l = lock(self.live);
            l.bytes_done = l.bytes_done.saturating_sub(uncount);
        }
        let failure = result.as_ref().err().copied();
        let _ = self
            .shared
            .writer
            .send(Message::FileResult { index, failure });
        self.resolve(index, result, true);
    }

    fn resolve(&mut self, index: u32, result: Result<PathBuf, FileFailure>, _reported: bool) {
        self.shared.requested().remove(&index);
        let path = self.offer.entries[index as usize].path.clone();
        let progress = self.journal.files.entry(index).or_default();
        progress.temp = None;
        {
            let mut l = lock(self.live);
            l.current = None;
            match &result {
                Ok(_) => l.files_done += 1,
                Err(failure) => l.fail(&path, *failure),
            }
        }
        match result {
            Ok(saved) => {
                progress.state = FileState::Done;
                progress.verified = self.offer.entries[index as usize].size;
                progress.saved = Some(saved);
            }
            Err(_) => {
                progress.state = FileState::Failed;
                progress.verified = 0;
            }
        }
        self.shared.permits.add_permits(1);
        self.shared.resolve_one();
    }
}

/// Moves a verified temporary file to its final place (FR-046).
async fn commit(temp: &Path, planned: &Path, conflict: ConflictPolicy) -> std::io::Result<PathBuf> {
    match conflict {
        ConflictPolicy::Replace => {
            if tokio::fs::metadata(planned).await.is_ok_and(|m| m.is_dir()) {
                return Err(std::io::Error::other("a folder has that name"));
            }
            tokio::fs::rename(temp, planned).await?;
            Ok(planned.to_path_buf())
        }
        ConflictPolicy::KeepBoth | ConflictPolicy::Skip => {
            let parent = planned.parent().unwrap_or(Path::new("."));
            let name = planned
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for _ in 0..8 {
                let target = naming::first_free(parent, &name);
                // A hard link never replaces an existing file; fall back to a
                // rename where links are unsupported (FAT, some Android storage).
                match tokio::fs::hard_link(temp, &target).await {
                    Ok(()) => {
                        let _ = tokio::fs::remove_file(temp).await;
                        return Ok(target);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(_) => {
                        if tokio::fs::symlink_metadata(&target).await.is_err() {
                            tokio::fs::rename(temp, &target).await?;
                            return Ok(target);
                        }
                    }
                }
            }
            Err(std::io::Error::other("no free name"))
        }
    }
}

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
