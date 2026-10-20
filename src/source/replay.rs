//! NMEA log replay, paced by the UTC time in the recording.
//!
//! Used for bench testing, CI fixtures and demos without hardware.

use super::{Tx, status};
use crate::config::AppConfig;
use crate::hub::SourceEvent;
use crate::nmea::Sentence;
use crate::nmea::fields::parse_time;
use crate::shared::LinkStatus;
use chrono::NaiveTime;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Longest pause honoured between epochs (gaps in a log are not replayed).
const MAX_PAUSE: Duration = Duration::from_secs(2);

fn epoch_time(line: &str) -> Option<NaiveTime> {
    let s = Sentence::parse(line, false).ok()?;
    match s.kind {
        "RMC" | "GGA" | "GNS" => parse_time(s.field(0)).ok(),
        _ => None,
    }
}

pub async fn run(config: &AppConfig, tx: Tx, cancel: CancellationToken) {
    let path = config.replay_file.clone().unwrap_or_default();
    let speed = config.replay_speed;
    loop {
        let file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) => {
                warn!("Cannot open replay file {path}: {e}");
                status(&tx, LinkStatus::Disconnected, format!("replay {path}"));
                return;
            }
        };
        info!("Replaying {path} at {speed}× speed");
        status(&tx, LinkStatus::Connected, format!("replay {path}"));
        let mut lines = BufReader::new(file).lines();
        let mut last: Option<NaiveTime> = None;
        loop {
            let line = tokio::select! {
                _ = cancel.cancelled() => return,
                l = lines.next_line() => l,
            };
            let Ok(Some(line)) = line else { break };
            if speed > 0.0
                && let Some(t) = epoch_time(&line)
            {
                if let Some(prev) = last.filter(|p| *p != t) {
                    let delta = (t - prev).to_std().unwrap_or_default().min(MAX_PAUSE);
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(delta.div_f64(speed)) => {}
                    }
                }
                last = Some(t);
            }
            let mut bytes = line.into_bytes();
            bytes.extend_from_slice(b"\r\n");
            if tx.send(SourceEvent::Bytes(bytes)).await.is_err() {
                return;
            }
        }
        if !config.replay_loop {
            info!("Replay finished");
            status(
                &tx,
                LinkStatus::Disconnected,
                format!("replay {path} (finished)"),
            );
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn replays_recorded_receiver_output() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/ublox-m8-cold-start.nmea"
        );
        let mut config = AppConfig::default();
        config.replay_file = Some(path.into());
        config.replay_speed = 0.0;
        let (tx, mut rx) = mpsc::channel(1024);
        run(&config, tx, CancellationToken::new()).await;
        let mut bytes = 0;
        let mut statuses = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            match ev {
                SourceEvent::Bytes(b) => bytes += b.len(),
                SourceEvent::Status(s, _) => statuses.push(s),
            }
        }
        assert!(bytes > 1000);
        assert_eq!(statuses, [LinkStatus::Connected, LinkStatus::Disconnected]);
    }

    #[test]
    fn epoch_time_only_for_timed_position_sentences() {
        assert!(epoch_time("$GNRMC,081942.00,V,,,,,,,,,,N*65").is_some());
        assert!(epoch_time("$GNVTG,,,,,,,,,N*2E").is_none());
    }
}
