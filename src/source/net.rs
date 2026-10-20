//! Network sources: TCP bridge, UDP datagrams and gpsd.

use super::{Backoff, Tx, status, wait};
use crate::config::AppConfig;
use crate::hub::SourceEvent;
use crate::receiver;
use crate::shared::LinkStatus;
use anyhow::{Context, Result};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A bridge that sends nothing for this long is treated as dead and reconnected.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

async fn connect(addr: &str) -> Result<TcpStream> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .context("connect timed out")??;
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// Pump bytes from a stream to the hub until EOF, idle timeout or cancellation.
/// Returns `Ok(true)` when cancelled / hub gone (stop), `Ok(false)` to reconnect.
async fn pump(stream: &mut TcpStream, tx: &Tx, cancel: &CancellationToken) -> Result<bool> {
    let mut buf = vec![0u8; 8192];
    loop {
        let n = tokio::select! {
            _ = cancel.cancelled() => return Ok(true),
            r = tokio::time::timeout(IDLE_TIMEOUT, stream.read(&mut buf)) => match r {
                Err(_) => {
                    warn!("No data for {IDLE_TIMEOUT:?}; reconnecting");
                    return Ok(false);
                }
                Ok(r) => r?,
            },
        };
        if n == 0 {
            warn!("Peer closed the connection");
            return Ok(false);
        }
        if tx
            .send(SourceEvent::Bytes(buf[..n].to_vec()))
            .await
            .is_err()
        {
            return Ok(true);
        }
    }
}

async fn reconnecting<F, Fut>(
    addr: String,
    tx: Tx,
    cancel: CancellationToken,
    label: &str,
    mut on_connect: F,
) where
    F: FnMut(TcpStream) -> Fut,
    Fut: std::future::Future<Output = Result<TcpStream>>,
{
    let mut backoff = Backoff::new();
    while !cancel.is_cancelled() {
        status(&tx, LinkStatus::Connecting, format!("{label} {addr}"));
        match connect(&addr).await {
            Ok(stream) => match on_connect(stream).await {
                Ok(mut stream) => {
                    backoff.reset();
                    info!("Connected to {label} {addr}");
                    status(&tx, LinkStatus::Connected, format!("{label} {addr}"));
                    match pump(&mut stream, &tx, &cancel).await {
                        Ok(true) => return,
                        Ok(false) => {}
                        Err(e) => warn!("{label} {addr} read error: {e:#}"),
                    }
                }
                Err(e) => warn!("{label} {addr} handshake failed: {e:#}"),
            },
            Err(e) => warn!("Cannot connect to {label} {addr}: {e:#}"),
        }
        status(&tx, LinkStatus::Disconnected, "");
        if !wait(&mut backoff, &cancel).await {
            return;
        }
    }
}

/// TCP client to a serial-to-network bridge. Init commands are forwarded,
/// since bridges such as io-to-net and ser2net are bidirectional.
pub async fn run_tcp(config: &AppConfig, tx: Tx, cancel: CancellationToken) {
    let addr = format!(
        "{}:{}",
        config.tcp_host.as_deref().unwrap_or_default(),
        config.tcp_port.unwrap_or_default()
    );
    // The bridge's serial speed cannot be changed from here: trim/rate only.
    let init = receiver::plan(
        config.effective_receiver(),
        config.effective_update_rate(),
        &config.init_commands,
        None,
    )
    .map(|p| p.commands)
    .unwrap_or_default();
    if config.effective_update_rate().is_some_and(|hz| hz > 5) {
        warn!("High update rates over TCP need the bridge's serial port at ≥ 115200 baud");
    }
    reconnecting(addr, tx, cancel, "TCP", |mut stream| {
        let init = init.clone();
        async move {
            for cmd in &init {
                stream.write_all(cmd).await?;
            }
            Ok(stream)
        }
    })
    .await;
}

/// gpsd client: asks gpsd for the raw NMEA stream of all devices.
pub async fn run_gpsd(config: &AppConfig, tx: Tx, cancel: CancellationToken) {
    reconnecting(
        config.gpsd_address(),
        tx,
        cancel,
        "gpsd",
        |mut stream| async move {
            stream
                .write_all(b"?WATCH={\"enable\":true,\"nmea\":true};\n")
                .await?;
            Ok(stream)
        },
    )
    .await;
}

/// UDP listener for NMEA datagrams (one or more sentences per datagram).
pub async fn run_udp(config: &AppConfig, tx: Tx, cancel: CancellationToken) {
    let mut backoff = Backoff::new();
    while !cancel.is_cancelled() {
        match UdpSocket::bind(&config.udp_bind).await {
            Ok(socket) => {
                backoff.reset();
                info!("Listening for NMEA datagrams on udp://{}", config.udp_bind);
                status(
                    &tx,
                    LinkStatus::Connecting,
                    format!("UDP {}", config.udp_bind),
                );
                let mut buf = vec![0u8; 65_536];
                let mut last_peer = None;
                loop {
                    let (n, peer) = tokio::select! {
                        _ = cancel.cancelled() => return,
                        r = socket.recv_from(&mut buf) => match r {
                            Ok(v) => v,
                            Err(e) => {
                                warn!("UDP receive error: {e}");
                                continue;
                            }
                        },
                    };
                    if last_peer != Some(peer) {
                        last_peer = Some(peer);
                        status(
                            &tx,
                            LinkStatus::Connected,
                            format!("UDP {} ← {peer}", config.udp_bind),
                        );
                    }
                    // Datagrams may omit the final line terminator.
                    let mut bytes = buf[..n].to_vec();
                    if !bytes.ends_with(b"\n") {
                        bytes.extend_from_slice(b"\r\n");
                    }
                    if tx.send(SourceEvent::Bytes(bytes)).await.is_err() {
                        return;
                    }
                }
            }
            Err(e) => warn!("Cannot bind UDP {}: {e}", config.udp_bind),
        }
        status(&tx, LinkStatus::Disconnected, "");
        if !wait(&mut backoff, &cancel).await {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    async fn collect(rx: &mut mpsc::Receiver<SourceEvent>, want: &str) -> String {
        let mut data = String::new();
        while !data.contains(want) {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
            {
                Some(SourceEvent::Bytes(b)) => data.push_str(&String::from_utf8_lossy(&b)),
                Some(SourceEvent::Status(..)) => {}
                None => break,
            }
        }
        data
    }

    #[tokio::test]
    async fn tcp_source_reads_and_sends_init() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut got = vec![0u8; 64];
            let n = s.read(&mut got).await.unwrap();
            s.write_all(b"$GPTXT,hello*00\r\n").await.unwrap();
            String::from_utf8_lossy(&got[..n]).to_string()
        });
        let mut config = AppConfig::default();
        config.tcp_host = Some("127.0.0.1".into());
        config.tcp_port = Some(port);
        config.init_commands = vec!["PMTK220,100".into()];
        let (tx, mut rx) = mpsc::channel(16);
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        let task = tokio::spawn(async move { run_tcp(&config, tx, c).await });
        assert!(collect(&mut rx, "hello").await.contains("$GPTXT"));
        assert_eq!(server.await.unwrap(), "$PMTK220,100*2F\r\n");
        cancel.cancel();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn gpsd_source_sends_watch() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut got = vec![0u8; 128];
            let n = s.read(&mut got).await.unwrap();
            s.write_all(b"{\"class\":\"VERSION\"}\n$GPGGA,x*00\r\n")
                .await
                .unwrap();
            String::from_utf8_lossy(&got[..n]).to_string()
        });
        let mut config = AppConfig::default();
        config.tcp_host = Some("127.0.0.1".into());
        config.tcp_port = Some(port);
        let (tx, mut rx) = mpsc::channel(16);
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        let task = tokio::spawn(async move { run_gpsd(&config, tx, c).await });
        collect(&mut rx, "$GPGGA").await;
        assert!(
            server
                .await
                .unwrap()
                .starts_with("?WATCH={\"enable\":true,\"nmea\":true}")
        );
        cancel.cancel();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn udp_source_receives_datagrams() {
        let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let mut config = AppConfig::default();
        config.udp_bind = addr.to_string();
        let (tx, mut rx) = mpsc::channel(16);
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        let task = tokio::spawn(async move { run_udp(&config, tx, c).await });
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let data = tokio::spawn(async move { collect(&mut rx, "$GPRMC").await });
        for _ in 0..20 {
            sender.send_to(b"$GPRMC,1*00", addr).await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            if data.is_finished() {
                break;
            }
        }
        assert!(data.await.unwrap().contains("$GPRMC,1*00\r\n"));
        cancel.cancel();
        task.await.unwrap();
    }
}
