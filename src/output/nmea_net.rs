//! NMEA-0183 network re-broadcast (TCP server and UDP sender).
//!
//! Port 10110 is the de-facto standard for NMEA over IP; OpenCPN, Signal K,
//! qtVlm, iNavX, Navionics and most chart plotters connect to it directly.
//! Sentences are forwarded verbatim (already checksum-verified).

use crate::shared::{BusEvent, Counters, Shared};
use anyhow::{Context, Result};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Shared accept loop with a client cap. `serve` handles one connection.
pub async fn accept_loop<F, Fut>(
    listener: TcpListener,
    max_clients: usize,
    shared: Arc<Shared>,
    cancel: CancellationToken,
    label: &'static str,
    serve: F,
) where
    F: Fn(TcpStream, Arc<Shared>, CancellationToken) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    loop {
        let (stream, peer) = tokio::select! {
            _ = cancel.cancelled() => return,
            r = listener.accept() => match r {
                Ok(v) => v,
                Err(e) => {
                    warn!("{label}: accept failed: {e}");
                    continue;
                }
            },
        };
        if active.load(Ordering::Relaxed) >= max_clients {
            debug!("{label}: rejecting {peer}, {max_clients} clients connected");
            continue;
        }
        let _ = stream.set_nodelay(true);
        info!("{label}: client {peer} connected");
        active.fetch_add(1, Ordering::Relaxed);
        Counters::inc(&shared.counters.clients);
        let fut = serve(stream, shared.clone(), cancel.child_token());
        let active = active.clone();
        let shared = shared.clone();
        tokio::spawn(async move {
            fut.await;
            active.fetch_sub(1, Ordering::Relaxed);
            shared.counters.clients.fetch_sub(1, Ordering::Relaxed);
            info!("{label}: client {peer} disconnected");
        });
    }
}

async fn serve_nmea(mut stream: TcpStream, shared: Arc<Shared>, cancel: CancellationToken) {
    let mut bus = shared.bus.subscribe();
    let (mut rd, mut wr) = stream.split();
    let mut sink = [0u8; 256];
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            // Clients may write (e.g. autopilot sentences); ignore input, detect close.
            r = rd.read(&mut sink) => if matches!(r, Ok(0) | Err(_)) { return },
            ev = bus.recv() => match ev {
                Ok(BusEvent::Nmea(line)) => {
                    let mut buf = Vec::with_capacity(line.len() + 2);
                    buf.extend_from_slice(line.as_bytes());
                    buf.extend_from_slice(b"\r\n");
                    if wr.write_all(&buf).await.is_err() { return; }
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
        }
    }
}

pub async fn run_tcp(
    addr: String,
    max_clients: usize,
    shared: Arc<Shared>,
    cancel: CancellationToken,
) -> Result<()> {
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding NMEA TCP server to {addr}"))?;
    info!("NMEA-0183 TCP server on tcp://{addr}");
    accept_loop(
        listener,
        max_clients,
        shared,
        cancel,
        "NMEA TCP",
        serve_nmea,
    )
    .await;
    Ok(())
}

pub async fn run_udp(target: String, shared: Arc<Shared>, cancel: CancellationToken) -> Result<()> {
    let target: std::net::SocketAddr = target.parse().context("nmea_udp_target")?;
    let bind = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind).await?;
    socket.set_broadcast(true)?;
    info!("NMEA-0183 UDP output → udp://{target}");
    let mut bus = shared.bus.subscribe();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            ev = bus.recv() => match ev {
                Ok(BusEvent::Nmea(line)) => {
                    let mut buf = Vec::with_capacity(line.len() + 2);
                    buf.extend_from_slice(line.as_bytes());
                    buf.extend_from_slice(b"\r\n");
                    if let Err(e) = socket.send_to(&buf, target).await {
                        debug!("UDP send failed: {e}");
                    }
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return Ok(()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::Info;
    use std::time::Duration;

    #[tokio::test]
    async fn tcp_clients_receive_sentences_and_cap_is_enforced() {
        let shared = Shared::new(Info::default(), false);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        tokio::spawn(accept_loop(
            listener,
            1,
            shared.clone(),
            cancel.clone(),
            "test",
            serve_nmea,
        ));

        let mut client = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut rejected = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        shared.publish(BusEvent::Nmea(Arc::from("$GPGGA,1*00")));
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(Duration::from_secs(2), client.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf[..n], b"$GPGGA,1*00\r\n");
        let n = tokio::time::timeout(Duration::from_secs(2), rejected.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n, 0, "over-cap client is closed");
        assert_eq!(Counters::get(&shared.counters.clients), 1);
        cancel.cancel();
    }

    #[tokio::test]
    async fn udp_sender_forwards_sentences() {
        let shared = Shared::new(Info::default(), false);
        let rx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = rx.local_addr().unwrap().to_string();
        let cancel = CancellationToken::new();
        tokio::spawn(run_udp(target, shared.clone(), cancel.clone()));
        tokio::time::sleep(Duration::from_millis(50)).await;
        shared.publish(BusEvent::Nmea(Arc::from("$GPRMC,1*00")));
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(Duration::from_secs(2), rx.recv(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf[..n], b"$GPRMC,1*00\r\n");
        cancel.cancel();
    }
}
