//! Transport comparison for the transport decision (PRD P0-012, ADR-001).
//!
//! Sends the same payload one way over (a) TCP + TLS 1.3, DexDrop's
//! transport, and (b) QUIC with `quinn`, the QUIC implementation Iroh is
//! built on. Both use the same self-signed Ed25519 identities, rustls with
//! the ring provider and TLS 1.3, and one stream; the receiver discards the
//! data, so this measures the transport alone (no hashing, no disk).
//!
//! Reports throughput and CPU time per GB for both ends together, which
//! matters for battery on phones.
//!
//! ```sh
//! cargo run --release -p dexdrop-bench -- [MiB] [rounds]
//! ```

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use dexdrop_core::Identity;
use dexdrop_net::tls::{self, TlsContext};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BLOCK: usize = 256 * 1024;

/// CPU time used by this process so far (user + system).
fn cpu_time() -> Duration {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: getrusage writes into the struct we own.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, (t.tv_usec as u32) * 1000);
    tv(usage.ru_utime) + tv(usage.ru_stime)
}

struct Sample {
    secs: f64,
    cpu: f64,
}

async fn tcp_tls(a: &TlsContext, b: &TlsContext, bytes: u64) -> Result<Sample> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let b = b.clone();
    let receiver = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await?;
        let (mut stream, _) = tls::accept(&b, tls::tcp_link(tcp)).await?;
        let mut buf = vec![0u8; BLOCK];
        let mut got = 0u64;
        while got < bytes {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            got += n as u64;
        }
        anyhow::Ok(got)
    });
    let (cpu0, t0) = (cpu_time(), Instant::now());
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let (mut stream, _) = tls::connect(a, tls::tcp_link(tcp)).await?;
    let block = vec![7u8; BLOCK];
    let mut sent = 0u64;
    while sent < bytes {
        stream.write_all(&block).await?;
        sent += BLOCK as u64;
    }
    stream.flush().await?;
    let got = receiver.await??;
    anyhow::ensure!(got >= bytes, "received {got} of {bytes}");
    Ok(Sample {
        secs: t0.elapsed().as_secs_f64(),
        cpu: (cpu_time() - cpu0).as_secs_f64(),
    })
}

fn quic_transport() -> Arc<quinn::TransportConfig> {
    // Windows large enough not to cap a fast local link.
    let mut transport = quinn::TransportConfig::default();
    transport.stream_receive_window(quinn::VarInt::from_u32(32 << 20));
    transport.receive_window(quinn::VarInt::from_u32(64 << 20));
    transport.send_window(64 << 20);
    Arc::new(transport)
}

async fn quic(a: &TlsContext, b: &TlsContext, bytes: u64) -> Result<Sample> {
    let server_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(b.server_config.clone())?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(server_crypto));
    server_config.transport_config(quic_transport());
    let server = quinn::Endpoint::server(server_config, "127.0.0.1:0".parse()?)?;
    let addr: SocketAddr = server.local_addr()?;
    let receiver = tokio::spawn(async move {
        let conn = server
            .accept()
            .await
            .ok_or_else(|| anyhow::anyhow!("no connection"))?
            .await?;
        let mut recv = conn.accept_uni().await?;
        let mut buf = vec![0u8; BLOCK];
        let mut got = 0u64;
        while let Some(n) = recv.read(&mut buf).await? {
            got += n as u64;
        }
        conn.close(0u32.into(), b"done");
        anyhow::Ok(got)
    });
    let client_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(a.client_config.clone())?;
    let mut client_config = quinn::ClientConfig::new(Arc::new(client_crypto));
    client_config.transport_config(quic_transport());
    let mut client = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
    client.set_default_client_config(client_config);

    let (cpu0, t0) = (cpu_time(), Instant::now());
    let conn = client.connect(addr, tls::SERVER_NAME)?.await?;
    let mut send = conn.open_uni().await?;
    let block = vec![7u8; BLOCK];
    let mut sent = 0u64;
    while sent < bytes {
        send.write_all(&block).await?;
        sent += BLOCK as u64;
    }
    send.finish()?;
    let got = receiver.await??;
    anyhow::ensure!(got >= bytes, "received {got} of {bytes}");
    let sample = Sample {
        secs: t0.elapsed().as_secs_f64(),
        cpu: (cpu_time() - cpu0).as_secs_f64(),
    };
    client.wait_idle().await;
    Ok(sample)
}

fn report(name: &str, bytes: u64, samples: &[Sample]) {
    let mb = bytes as f64 / 1e6;
    let mut rates: Vec<f64> = samples.iter().map(|s| mb / s.secs).collect();
    rates.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = rates[rates.len() / 2];
    let cpu_per_gb: f64 = samples
        .iter()
        .map(|s| s.cpu / (bytes as f64 / 1e9))
        .sum::<f64>()
        / samples.len() as f64;
    println!(
        "{name:<16} median {median:>7.0} MB/s  (min {:>5.0}, max {:>5.0})  CPU {cpu_per_gb:>5.2} s/GB (both ends)",
        rates[0],
        rates[rates.len() - 1]
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mib: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1024);
    let rounds: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(5);
    let bytes = mib * 1024 * 1024;
    let a = TlsContext::new(&Identity::generate()?)?;
    let b = TlsContext::new(&Identity::generate()?)?;
    println!(
        "{mib} MiB one way, {rounds} rounds, loopback, TLS 1.3 (ring), Ed25519 identities, one stream, no disk"
    );
    let mut tcp_samples = Vec::new();
    let mut quic_samples = Vec::new();
    // Interleaved so both see the same machine conditions.
    for _ in 0..rounds {
        tcp_samples.push(tcp_tls(&a, &b, bytes).await?);
        quic_samples.push(quic(&a, &b, bytes).await?);
    }
    report("TCP + TLS 1.3", bytes, &tcp_samples);
    report("QUIC (quinn)", bytes, &quic_samples);
    Ok(())
}
