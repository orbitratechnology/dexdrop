//! Sending referenced items (docs/FILE_SELECTION.md): a photo in the
//! library, a document of a provider, read where it is through its
//! provider, never copied. The provider here is not a file at all: a stream
//! that hands out small reads, as PhotoKit's does, and counts what it gave.

mod common;

use std::{
    io::{self, Read},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use common::Node;
use dexdrop_core::{
    asset::{
        AssetId, AssetPath, AssetProvider, AssetSource, MediaItem, MediaType, SourceReader, Stat,
        register,
    },
    naming::ConflictPolicy,
};
use dexdrop_net::{Command, Event, TransferState};

/// Item `token`'s bytes: a pattern of its length, so a byte out of place
/// shows.
fn content(len: u64) -> impl Fn(u64) -> u8 {
    move |i| ((i * 31 + len) % 251) as u8
}

struct Streams {
    scheme: &'static str,
    /// Bytes handed out so far, over every open.
    read: Arc<AtomicU64>,
    opens: Arc<AtomicU64>,
}

struct Stream {
    len: u64,
    pos: u64,
    read: Arc<AtomicU64>,
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Small, uneven pieces, as a platform stream delivers them.
        let n = (self.len - self.pos)
            .min(buf.len() as u64)
            .min(48 * 1024 + 7) as usize;
        let at = content(self.len);
        for (k, b) in buf[..n].iter_mut().enumerate() {
            *b = at(self.pos + k as u64);
        }
        self.pos += n as u64;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

impl SourceReader for Stream {
    fn stat(&self) -> io::Result<Stat> {
        Ok(Stat {
            is_dir: false,
            len: self.len,
            mtime: Some(1_700_000_000),
        })
    }
}

/// `token` is the item's size in bytes.
fn size_of(id: &AssetId) -> u64 {
    id.token().trim_end_matches(".mov").parse().unwrap_or(0)
}

impl AssetProvider for Streams {
    fn scheme(&self) -> &str {
        self.scheme
    }
    fn metadata(&self, id: &AssetId) -> io::Result<MediaItem> {
        Ok(MediaItem {
            id: id.clone(),
            name: id.token(),
            mime_type: "video/quicktime".into(),
            size: Some(size_of(id)),
            media_type: MediaType::Video,
            source: AssetSource::IosPhotoLibrary,
            created_at: None,
            modified_at: Some(1_700_000_000),
            duration: None,
            width: None,
            height: None,
            album: None,
        })
    }
    fn open(&self, id: &AssetId) -> io::Result<Box<dyn SourceReader>> {
        self.opens.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(Stream {
            len: size_of(id),
            pos: 0,
            read: self.read.clone(),
        }))
    }
}

fn accept(id: [u8; 16]) -> Command {
    Command::Respond {
        id,
        accept: true,
        trust: false,
        conflict: ConflictPolicy::KeepBoth,
    }
}

fn reference(scheme: &str, len: u64) -> std::path::PathBuf {
    let name = format!("{len}.mov");
    AssetPath::new(AssetId::new(scheme, &name), &name).to_path()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn referenced_items_stream_from_their_provider() {
    let read = Arc::new(AtomicU64::new(0));
    let opens = Arc::new(AtomicU64::new(0));
    register(Arc::new(Streams {
        scheme: "test-streams",
        read: read.clone(),
        opens: opens.clone(),
    }));
    let a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let big = 6 * 1024 * 1024 + 123;
    let small = 2_000;
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![
            reference("test-streams", big),
            reference("test-streams", small),
        ],
    });
    let request = b.wait_request().await;
    assert_eq!(request.files, 2);
    assert_eq!(request.total_bytes, big + small);
    // Offering read nothing: the manifest came from metadata.
    assert_eq!(read.load(Ordering::Relaxed), 0);
    b.send(accept(request.id));
    let done = b.wait_state("completed", |s| s.is_finished()).await;
    assert_eq!(done.state, TransferState::Completed, "{done:?}");
    for len in [big, small] {
        let got = std::fs::read(b.downloads().join(format!("{len}.mov"))).unwrap();
        assert_eq!(got.len() as u64, len);
        let at = content(len);
        assert!(
            got.iter().enumerate().all(|(i, b)| *b == at(i as u64)),
            "{len}.mov differs"
        );
    }
    assert!(opens.load(Ordering::Relaxed) >= 2);
    // The sender holds nothing of them: no file, no copy in its state.
    let copies = walk(&a.state())
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(".mov"))
        .count();
    assert_eq!(copies, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_referenced_item_resumes_after_a_pause() {
    let read = Arc::new(AtomicU64::new(0));
    register(Arc::new(Streams {
        scheme: "test-streams-pause",
        read: read.clone(),
        opens: Arc::new(AtomicU64::new(0)),
    }));
    let mut a = Node::start_with("Alice", |c| {
        c.chunk_size = 64 * 1024;
        c.max_send_rate = Some(16_000_000);
    })
    .await;
    let mut b = Node::start("Bob").await;
    let size = 24 * 1024 * 1024;
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![reference("test-streams-pause", size)],
    });
    let request = b.wait_request().await;
    let id = request.id;
    b.send(accept(id));
    a.wait("some progress", |e| match e {
        Event::Transfer(t) if t.bytes_done > 2 * 1024 * 1024 => Some(()),
        _ => None,
    })
    .await;
    a.send(Command::Pause(id));
    b.wait_state("receiver paused", |s| {
        matches!(s, TransferState::Paused { by_peer: true })
    })
    .await;
    a.send(Command::Resume(id));
    let end = b
        .wait("receiver finished", |e| match e {
            Event::Transfer(t) if t.id == id && t.state.is_finished() => Some(t.clone()),
            _ => None,
        })
        .await;
    assert_eq!(end.state, TransferState::Completed, "{end:?}");
    let got = std::fs::read(b.downloads().join(format!("{size}.mov"))).unwrap();
    let at = content(size);
    assert_eq!(got.len() as u64, size);
    assert!(got.iter().enumerate().all(|(i, b)| *b == at(i as u64)));
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
