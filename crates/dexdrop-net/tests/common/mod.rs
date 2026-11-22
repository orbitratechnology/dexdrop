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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
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
