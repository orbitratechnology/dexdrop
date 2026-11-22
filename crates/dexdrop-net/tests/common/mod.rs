#![allow(dead_code)]

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use dexdrop_net::{Command, EngineConfig, Event, Status, Target, TransferSnapshot, TransferState};
use tokio::sync::mpsc::UnboundedReceiver;

pub struct Node {
    pub engine: dexdrop_net::EngineHandle,
    pub events: UnboundedReceiver<Event>,
    pub status: Status,
    pub dir: tempfile::TempDir,
    /// How long [`Node::wait`] waits for an event.
    pub timeout: Duration,
    _done: tokio::sync::oneshot::Receiver<()>,
}

impl Node {
    pub async fn start(name: &str) -> Node {
        Self::start_with(name, |_| {}).await
    }

    pub async fn start_with(name: &str, tweak: impl FnOnce(&mut EngineConfig)) -> Node {
        let dir = tempfile::tempdir().unwrap();
        let mut config = EngineConfig::new(
            dir.path().join("state"),
            dir.path().join("Downloads"),
            "linux",
        );
        config.discovery = false;
        config.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
        config.default_device_name = name.to_owned();
        tweak(&mut config);
        Self::start_in(dir, config).await
    }

    pub async fn start_in(dir: tempfile::TempDir, config: EngineConfig) -> Node {
        let (engine, mut events, done) = dexdrop_net::spawn(config).await.unwrap();
        let status = loop {
            match events.recv().await.unwrap() {
                Event::Status(status) => break status,
                _ => continue,
            }
        };
        Node {
            engine,
            events,
            status,
            dir,
            // CI's Windows disks flush slowly while other tests run.
            timeout: Duration::from_secs(if cfg!(windows) { 90 } else { 30 }),
            _done: done,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.status.port)
    }

    pub fn target(&self) -> Target {
        Target::Address {
            addr: self.addr(),
            peer_id: Some(self.status.peer_id),
        }
    }

    pub fn downloads(&self) -> PathBuf {
        self.dir.path().join("Downloads")
    }

    pub fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    pub fn send(&self, command: Command) {
        self.engine.send(command);
    }

    /// Waits for an event matching `pred`.
    pub async fn wait<T>(&mut self, what: &str, mut pred: impl FnMut(&Event) -> Option<T>) -> T {
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            let event = tokio::time::timeout_at(deadline, self.events.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
                .expect("engine stopped");
            if let Some(found) = pred(&event) {
                return found;
            }
        }
    }

    pub async fn wait_state(
        &mut self,
        what: &str,
        pred: impl Fn(&TransferState) -> bool,
    ) -> TransferSnapshot {
        self.wait(what, |e| match e {
            Event::Transfer(t) if pred(&t.state) => Some(t.clone()),
            _ => None,
        })
        .await
    }

    pub async fn wait_request(&mut self) -> dexdrop_net::IncomingRequest {
        self.wait("an incoming request", |e| match e {
            Event::Request(r) => Some(r.clone()),
            _ => None,
        })
        .await
    }
}

pub fn write_file(path: &Path, len: usize, seed: u8) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let data: Vec<u8> = (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

/// A TCP proxy that forwards to `upstream` and, once `cut_after` bytes went
/// through, resets every connection once, as a dropped Wi-Fi link would.
/// Later connections pass untouched.
pub struct CuttingProxy {
    pub addr: SocketAddr,
    pub cuts: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Bytes forwarded towards the upstream, over all connections.
    pub moved: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl CuttingProxy {
    pub async fn start(upstream: SocketAddr, cut_after: u64) -> CuttingProxy {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cuts = Arc::new(AtomicU32::new(0));
        let armed = Arc::new(AtomicBool::new(true));
        let moved = Arc::new(AtomicU64::new(0));
        let cuts2 = cuts.clone();
        let moved_out = moved.clone();
        tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                let Ok(server) = tokio::net::TcpStream::connect(upstream).await else {
                    continue;
                };
                let (armed, moved, cuts) = (armed.clone(), moved.clone(), cuts2.clone());
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let (mut cr, mut cw) = client.into_split();
                    let (mut sr, mut sw) = server.into_split();
                    let cut = Arc::new(tokio::sync::Notify::new());
                    let up = {
                        let (armed, moved, cuts, cut) =
                            (armed.clone(), moved.clone(), cuts.clone(), cut.clone());
                        async move {
                            let mut buf = vec![0u8; 64 * 1024];
                            loop {
                                let n = match cr.read(&mut buf).await {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => n,
                                };
                                if sw.write_all(&buf[..n]).await.is_err() {
                                    return;
                                }
                                let total = moved.fetch_add(n as u64, Ordering::SeqCst) + n as u64;
                                if total >= cut_after && armed.swap(false, Ordering::SeqCst) {
                                    cuts.fetch_add(1, Ordering::SeqCst);
                                    cut.notify_waiters();
                                    return;
                                }
                            }
                        }
                    };
                    let down = async move {
                        let mut buf = vec![0u8; 64 * 1024];
                        loop {
                            let n = match sr.read(&mut buf).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => n,
                            };
                            if cw.write_all(&buf[..n]).await.is_err() {
                                return;
                            }
                        }
                    };
                    tokio::select! {
                        _ = up => {}
                        _ = down => {}
                        _ = cut.notified() => {}
                    }
                    // Dropping both halves of both sides closes them abruptly.
                });
            }
        });
        CuttingProxy {
            addr,
            cuts,
            moved: moved_out,
        }
    }
}
