//! `dexdrop`: send and receive from a terminal, list nearby devices and
//! measure throughput. It drives the same engine as the app.

use std::{
    io::{BufRead, Write},
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use dexdrop_core::{format, invitation, naming::ConflictPolicy};
use dexdrop_net::{Command, EngineConfig, Event, NoticeKind, Target, TransferState};

#[derive(Parser)]
#[command(
    name = "dexdrop",
    version,
    about = "Fast, private, local file transfer between your devices."
)]
struct Cli {
    /// Where DexDrop keeps its identity and state.
    #[arg(long, global = true, env = "DEXDROP_STATE")]
    state: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Wait for transfers and ask before accepting each one.
    Receive {
        /// Where received files go.
        #[arg(long, short)]
        dir: Option<PathBuf>,
        /// Accept every transfer without asking (for scripts).
        #[arg(long)]
        yes: bool,
        /// Port to listen on (default: any free port).
        #[arg(long)]
        port: Option<u16>,
        /// The name other devices see.
        #[arg(long)]
        name: Option<String>,
        /// Also show a pairing code and an invitation.
        #[arg(long)]
        invite: bool,
    },
    /// Send files or folders.
    Send {
        /// A nearby device's name or ID prefix, an invitation (DEXDROP1:…) or
        /// a picture of its QR code (.png/.jpg/.webp), a pairing code
        /// (K7P2-QX9M) or an address (host:port[#peer-id]).
        to: String,
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// List devices nearby.
    Peers {
        /// Seconds to listen.
        #[arg(long, default_value_t = 3)]
        wait: u64,
    },
    /// Show this device's identity.
    Id,
    /// Measure transfer throughput between two local engines.
    Bench {
        /// Size of the test file, in MB.
        #[arg(long, default_value_t = 512)]
        mb: u64,
        /// Chunk size, in KiB.
        #[arg(long, default_value_t = 4096)]
        chunk_kib: u32,
        /// Send a folder of this many small files instead of one file.
        #[arg(long, default_value_t = 0)]
        files: u64,
        /// Size of each small file, in KiB (with --files).
        #[arg(long, default_value_t = 16)]
        file_kib: u64,
    },
}

fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "android" => "android",
        "ios" => "ios",
        _ => "linux",
    }
}

fn default_state() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("dexdrop-cli")
}

fn default_downloads() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join("Downloads").join("DexDrop"))
        .unwrap_or_else(|| PathBuf::from("DexDrop"))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let state = cli.state.unwrap_or_else(default_state);
    match cli.command {
        Cmd::Receive {
            dir,
            yes,
            port,
            name,
            invite,
        } => receive(state, dir, yes, port, name, invite).await,
        Cmd::Send { to, paths } => send(state, to, paths).await,
        Cmd::Peers { wait } => peers(state, wait).await,
        Cmd::Id => id(state).await,
        Cmd::Bench {
            mb,
            chunk_kib,
            files,
            file_kib,
        } => bench(mb, chunk_kib, files, file_kib).await,
    }
}

fn config(state: PathBuf, downloads: PathBuf) -> EngineConfig {
    EngineConfig::new(state, downloads, platform())
}

fn progress_line(t: &dexdrop_net::TransferSnapshot) -> String {
    let mut line = format!(
        "{:>5.1}%  {} of {}",
        t.progress() * 100.0,
        format::bytes(t.bytes_done),
        format::bytes(t.total_bytes)
    );
    if let Some(rate) = t.rate {
        line += &format!("  {}", format::rate(rate));
    }
    if let Some(eta) = t.eta {
        line += &format!("  {} left", format::duration(eta));
    }
    line
}

async fn receive(
    state: PathBuf,
    dir: Option<PathBuf>,
    yes: bool,
    port: Option<u16>,
    name: Option<String>,
    invite: bool,
) -> Result<()> {
    let mut cfg = config(state, dir.unwrap_or_else(default_downloads));
    cfg.port = port;
    if let Some(name) = name {
        cfg.default_device_name = name;
    }
    let (engine, mut events, _done) = dexdrop_net::spawn(cfg).await?;
    if invite {
        engine.send(Command::CreateCode);
        engine.send(Command::CreateInvitation);
    }
    let (answers_tx, mut answers) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if answers_tx.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        let Some(event) = events.recv().await else {
            break;
        };
        match event {
            Event::Status(s) => {
                let addrs: Vec<String> = s
                    .addrs
                    .iter()
                    .map(|a| SocketAddr::new(*a, s.port).to_string())
                    .collect();
                println!("{} ({}) is ready to receive.", s.device_name, s.fingerprint);
                println!(
                    "  listening on {}",
                    if addrs.is_empty() {
                        format!("port {}", s.port)
                    } else {
                        addrs.join(", ")
                    }
                );
                println!("  peer ID {}", s.peer_id);
                if let dexdrop_net::DiscoveryStatus::Unavailable(why) = s.discovery {
                    println!("  nearby discovery is unavailable: {why}");
                }
            }
            Event::Code { code, .. } => {
                println!("Pairing code: {}", invitation::display_code(&code))
            }
            Event::Invitation(inv) => println!("Invitation (valid 10 minutes):\n  {}", inv.text),
            Event::Request(r) => {
                println!(
                    "\n{} ({}) wants to send {} — {}{}",
                    r.peer_name,
                    r.fingerprint,
                    r.label,
                    dexdrop_net::summary(r.files, r.folders, r.total_bytes),
                    if r.trusted { " [trusted]" } else { "" }
                );
                println!(
                    "  verification number: {}  → saving to {}",
                    r.sas,
                    r.destination.display()
                );
                if !r.enough_space() {
                    println!("  warning: not enough free space");
                }
                let accept = if yes {
                    true
                } else {
                    print!("Accept? [y/N] ");
                    std::io::stdout().flush().ok();
                    answers
                        .recv()
                        .await
                        .is_some_and(|a| a.trim().eq_ignore_ascii_case("y"))
                };
                engine.send(Command::Respond {
                    id: r.id,
                    accept,
                    trust: false,
                    conflict: ConflictPolicy::KeepBoth,
                });
            }
            Event::Transfer(t) => match &t.state {
                TransferState::Transferring => eprint!("\r{}        ", progress_line(&t)),
                s if s.is_finished() => {
                    eprintln!();
                    println!("{}: {} from {}", s.label(), t.label, t.peer_name);
                    for (path, why) in &t.failures {
                        println!("  {path}: {why}");
                    }
                }
                _ => {}
            },
            Event::Notice {
                kind: NoticeKind::Error,
                text,
            } => eprintln!("error: {text}"),
            _ => {}
        }
    }
    Ok(())
}

async fn resolve_target(
    to: &str,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>,
) -> Result<Target> {
    // A picture of a QR invitation (a screenshot or a photo).
    let as_path = std::path::Path::new(to);
    if as_path.is_file()
        && let Some(ext) = as_path.extension().and_then(|e| e.to_str())
        && matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp"
        )
    {
        let bytes = std::fs::read(as_path).with_context(|| format!("reading {to}"))?;
        let text = invitation::read_qr_image(&bytes).map_err(|e| anyhow::anyhow!(e))?;
        return Ok(Target::Invitation(text));
    }
    if to.to_ascii_uppercase().starts_with(invitation::PREFIX) {
        return Ok(Target::Invitation(to.to_owned()));
    }
    let (addr_part, id_part) = match to.split_once('#') {
        Some((a, i)) => (a, Some(i)),
        None => (to, None),
    };
    if let Ok(addr) = addr_part.parse::<SocketAddr>() {
        let peer_id = id_part
            .map(|i| i.parse())
            .transpose()
            .context("invalid peer ID after #")?;
        return Ok(Target::Address { addr, peer_id });
    }
    if let Ok(mut addrs) = tokio::net::lookup_host(addr_part).await
        && let Some(addr) = addrs.next()
    {
        let peer_id = id_part
            .map(|i| i.parse())
            .transpose()
            .context("invalid peer ID after #")?;
        return Ok(Target::Address { addr, peer_id });
    }
    // A nearby device by name or ID prefix.
    let wanted = to.to_lowercase();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout_at(deadline, events.recv()).await {
        if let Event::Peers(peers) = event {
            if let Some(p) = peers.iter().find(|p| {
                p.name.to_lowercase() == wanted
                    || p.peer_id.to_string().starts_with(&wanted)
                    || p.fingerprint == wanted
            }) {
                return Ok(Target::Peer(p.peer_id));
            }
            seen = peers;
        }
    }
    if invitation::normalize_code(to).is_some() {
        return Ok(Target::Code(to.to_owned()));
    }
    let names: Vec<_> = seen.iter().map(|p| p.name.clone()).collect();
    bail!(
        "no nearby device called {to:?} (found: {})",
        if names.is_empty() {
            "none".into()
        } else {
            names.join(", ")
        }
    )
}

async fn send(state: PathBuf, to: String, paths: Vec<PathBuf>) -> Result<()> {
    let (engine, mut events, _done) =
        dexdrop_net::spawn(config(state, default_downloads())).await?;
    let target = resolve_target(&to, &mut events).await?;
    engine.send(Command::SendFiles { target, paths });
    let mut announced = false;
    // Transfers restored from earlier sessions also report; follow only the
    // one this command starts (it is announced while preparing).
    let mut ours = None;
    while let Some(event) = events.recv().await {
        match event {
            Event::Transfer(t) => {
                if ours.is_none() && t.state == TransferState::Preparing {
                    ours = Some(t.id);
                }
                if ours != Some(t.id) {
                    continue;
                }
                if let (Some(sas), false) = (&t.sas, announced) {
                    println!("Connected to {} — verification number {sas}", t.peer_name);
                    announced = true;
                }
                match &t.state {
                    TransferState::Waiting => eprint!("\rWaiting for {} to accept…", t.peer_name),
                    TransferState::Transferring => eprint!("\r{}        ", progress_line(&t)),
                    TransferState::Interrupted { reason, .. } => eprint!("\r{reason}        "),
                    s if s.is_finished() => {
                        eprintln!();
                        println!("{}: {}", s.label(), t.label);
                        for (path, why) in &t.failures {
                            println!("  {path}: {why}");
                        }
                        if *s == TransferState::Completed {
                            return Ok(());
                        }
                        bail!("transfer did not complete");
                    }
                    _ => {}
                }
            }
            Event::Notice {
                kind: NoticeKind::Error,
                text,
            } => bail!(text),
            _ => {}
        }
    }
    Ok(())
}

async fn peers(state: PathBuf, wait: u64) -> Result<()> {
    let (_engine, mut events, _done) =
        dexdrop_net::spawn(config(state, default_downloads())).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(wait);
    let mut last = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout_at(deadline, events.recv()).await {
        match event {
            Event::Peers(peers) => last = peers,
            Event::Status(s) => {
                if let dexdrop_net::DiscoveryStatus::Unavailable(why) = s.discovery {
                    bail!("nearby discovery is unavailable: {why}");
                }
            }
            _ => {}
        }
    }
    if last.is_empty() {
        println!("No devices found nearby.");
    }
    for p in last {
        println!(
            "{:<28} {:<8} {}{}",
            p.name,
            p.platform,
            p.fingerprint,
            if p.trusted { "  trusted" } else { "" }
        );
    }
    Ok(())
}

async fn id(state: PathBuf) -> Result<()> {
    let mut cfg = config(state, default_downloads());
    cfg.discovery = false;
    let (_engine, mut events, _done) = dexdrop_net::spawn(cfg).await?;
    while let Some(event) = events.recv().await {
        if let Event::Status(s) = event {
            println!(
                "{}\n  fingerprint {}\n  peer ID {}",
                s.device_name, s.fingerprint, s.peer_id
            );
            break;
        }
    }
    Ok(())
}

/// Two engines in one process over loopback TLS: the end-to-end ceiling of
/// the protocol on this machine (disk, hashing, encryption included).
async fn bench(mb: u64, chunk_kib: u32, files: u64, file_kib: u64) -> Result<()> {
    let dir = tempfile_dir()?;
    let block: Vec<u8> = (0..1024 * 1024)
        .map(|i| ((i * 2654435761u64 as usize) >> 13) as u8)
        .collect();
    let (src, bytes) = if files > 0 {
        // A folder of many small files, 100 per subfolder.
        let root = dir.join("many");
        let size = (file_kib * 1024) as usize;
        for i in 0..files {
            let sub = root.join(format!("d{:04}", i / 100));
            if i % 100 == 0 {
                std::fs::create_dir_all(&sub)?;
            }
            // Distinct contents per file, any size: the block, rotated.
            let shift = (i as usize * 7919) % block.len();
            let data: Vec<u8> = block
                .iter()
                .cycle()
                .skip(shift)
                .take(size)
                .copied()
                .collect();
            std::fs::write(sub.join(format!("f{i:06}.bin")), &data)?;
        }
        (root, files * size as u64)
    } else {
        let src = dir.join("bench.bin");
        let mut f = std::io::BufWriter::new(std::fs::File::create(&src)?);
        for _ in 0..mb {
            f.write_all(&block)?;
        }
        (src, mb * 1024 * 1024)
    };
    let make = |name: &str| {
        let mut c = config(dir.join(name).join("state"), dir.join(name).join("dl"));
        c.discovery = false;
        c.listen = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        c.chunk_size = chunk_kib * 1024;
        c
    };
    let (a, mut a_events, _a) = dexdrop_net::spawn(make("a")).await?;
    let (b, mut b_events, _b) = dexdrop_net::spawn(make("b")).await?;
    let b_status = loop {
        if let Some(Event::Status(s)) = b_events.recv().await {
            break s;
        }
    };
    let addr = SocketAddr::from(([127, 0, 0, 1], b_status.port));
    let started = Instant::now();
    a.send(Command::SendFiles {
        target: Target::Address {
            addr,
            peer_id: Some(b_status.peer_id),
        },
        paths: vec![src.clone()],
    });
    let _ = &src;
    let mut accepted_at = None;
    loop {
        tokio::select! {
            Some(e) = b_events.recv() => if let Event::Request(r) = e {
                accepted_at = Some(Instant::now());
                b.send(Command::Respond { id: r.id, accept: true, trust: false, conflict: ConflictPolicy::KeepBoth });
            },
            Some(e) = a_events.recv() => if let Event::Transfer(t) = e && t.state.is_finished() {
                let elapsed = accepted_at.map(|t| t.elapsed()).unwrap_or_else(|| started.elapsed());
                if files > 0 {
                    println!(
                        "{}: {} files of {} KiB in {:.2} s = {:.0} files/s, {} (peak memory {})",
                        t.state.label(),
                        files,
                        file_kib,
                        elapsed.as_secs_f64(),
                        files as f64 / elapsed.as_secs_f64(),
                        format::rate(bytes as f64 / elapsed.as_secs_f64()),
                        peak_memory()
                    );
                    break;
                }
                println!(
                    "{}: {} in {:.2} s = {} (chunk {} KiB, TCP + TLS 1.3, BLAKE3 per chunk and per file)",
                    t.state.label(),
                    format::bytes(bytes),
                    elapsed.as_secs_f64(),
                    format::rate(bytes as f64 / elapsed.as_secs_f64()),
                    chunk_kib
                );
                break;
            },
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Peak resident memory of this process (both engines), where the OS says.
fn peak_memory() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        })
        .map(|kib| format::bytes(kib * 1024))
        .unwrap_or_else(|| "unknown".into())
}

fn tempfile_dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "dexdrop-bench-{}",
        dexdrop_core::hex(&dexdrop_core::random_bytes::<4>())
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
