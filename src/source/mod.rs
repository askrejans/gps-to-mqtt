//! Input sources. Each source runs in its own task, reconnects with backoff,
//! reports [`LinkStatus`] and forwards raw bytes to the hub — framing and
//! validation happen in exactly one place.

mod net;
mod replay;
pub mod serial;

use crate::config::{AppConfig, ConnectionType};
use crate::hub::SourceEvent;
use crate::shared::LinkStatus;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub type Tx = mpsc::Sender<SourceEvent>;

/// Exponential reconnect backoff: 1 s, 2 s, 4 s … capped at 30 s.
pub struct Backoff {
    next: Duration,
}

impl Backoff {
    const MIN: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(30);

    pub fn new() -> Self {
        Self { next: Self::MIN }
    }

    pub fn reset(&mut self) {
        self.next = Self::MIN;
    }

    pub fn next_delay(&mut self) -> Duration {
        let d = self.next;
        self.next = (self.next * 2).min(Self::MAX);
        d
    }
}

pub fn status(tx: &Tx, status: LinkStatus, detail: impl Into<String>) {
    let _ = tx.try_send(SourceEvent::Status(status, detail.into()));
}

/// Sleep for the backoff delay unless cancelled. Returns `false` when cancelled.
pub async fn wait(backoff: &mut Backoff, cancel: &CancellationToken) -> bool {
    let delay = backoff.next_delay();
    tracing::info!("Reconnecting in {:?}", delay);
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        _ = cancel.cancelled() => false,
    }
}

pub fn spawn(config: &AppConfig, tx: Tx, cancel: CancellationToken) {
    let config = config.clone();
    match config.connection_type {
        ConnectionType::Serial => {
            tokio::task::spawn_blocking(move || serial::run(&config, tx, cancel));
        }
        ConnectionType::Tcp => {
            tokio::spawn(async move { net::run_tcp(&config, tx, cancel).await });
        }
        ConnectionType::Udp => {
            tokio::spawn(async move { net::run_udp(&config, tx, cancel).await });
        }
        ConnectionType::Gpsd => {
            tokio::spawn(async move { net::run_gpsd(&config, tx, cancel).await });
        }
        ConnectionType::File => {
            tokio::spawn(async move { replay::run(&config, tx, cancel).await });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let mut b = Backoff::new();
        let seq: Vec<u64> = (0..7).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(seq, [1, 2, 4, 8, 16, 30, 30]);
        b.reset();
        assert_eq!(b.next_delay().as_secs(), 1);
    }
}
