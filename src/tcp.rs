//! TCP connection task for GPS data from a serial-to-TCP bridge.
//!
//! Connects to a TCP bridge such as [io-to-net](https://github.com/askrejans/io-to-net)
//! which forwards a serial GPS device over the network.
//!
//! Works with both bridge framing modes:
//! - **Line mode** (`frame_mode = "line"` in io-to-net): each NMEA sentence arrives
//!   as a complete newline-terminated frame.
//! - **Raw/stream mode**: arbitrary byte chunks are received; the internal buffer
//!   accumulates them and extracts complete NMEA sentences whenever a `\n` is found.
//!
//! NMEA sentences are always terminated with `\r\n` per the spec, so both modes
//! produce valid output from the same buffered-line-detection logic.
//!
//! The task automatically reconnects with backoff when the connection is
//! dropped or refused, mirroring the resilience of the serial task.

use crate::config::AppConfig;
use crate::metrics::NMEA_SENTENCES_TOTAL;
use crate::parser::{GpsEvent, parse_nmea_sentence};
use anyhow::Result;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_CONSECUTIVE_ERRORS: u32 = 3;
const ERROR_BACKOFF_DELAY: Duration = Duration::from_secs(10);

/// Spawn the TCP reading task.
pub async fn spawn_tcp_task(
    config: AppConfig,
    event_tx: mpsc::Sender<GpsEvent>,
    cancel: CancellationToken,
) -> Result<()> {
    let host = config.tcp_host.clone().unwrap_or_default();
    let port = config.tcp_port.unwrap_or(0);

    tokio::spawn(async move {
        run_tcp_loop(&host, port, event_tx, cancel).await;
    });

    Ok(())
}

/// Run the TCP connection loop with automatic reconnection.
async fn run_tcp_loop(
    host: &str,
    port: u16,
    event_tx: mpsc::Sender<GpsEvent>,
    cancel: CancellationToken,
) {
    let mut consecutive_errors = 0u32;
    let addr = format!("{}:{}", host, port);

    loop {
        if cancel.is_cancelled() {
            info!("TCP loop shutting down");
            break;
        }

        info!("Connecting to TCP bridge: {}", addr);

        match TcpStream::connect(&addr).await {
            Ok(stream) => {
                info!("TCP bridge connection established");
                consecutive_errors = 0;

                match read_from_stream(stream, &event_tx, &cancel).await {
                    Ok(_) => {
                        // Graceful EOF or cancellation — exit without reconnecting
                        if cancel.is_cancelled() {
                            info!("TCP loop exiting (cancelled)");
                        } else {
                            // EOF from bridge — bridge closed the connection, reconnect
                            warn!("TCP bridge closed connection (EOF), reconnecting...");
                            consecutive_errors += 1;
                        }
                    }
                    Err(e) => {
                        if e.to_string().contains("channel closed") || cancel.is_cancelled() {
                            info!("TCP loop exiting ({})", e);
                            break;
                        }
                        error!("TCP read error: {}", e);
                        consecutive_errors += 1;
                    }
                }
            }
            Err(e) => {
                if cancel.is_cancelled() {
                    break;
                }
                error!("Failed to connect to TCP bridge {}: {}", addr, e);
                consecutive_errors += 1;
            }
        }

        if cancel.is_cancelled() {
            break;
        }

        let delay = if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
            warn!("Max consecutive TCP errors reached, using extended backoff");
            ERROR_BACKOFF_DELAY
        } else {
            RECONNECT_DELAY
        };

        warn!("Reconnecting to TCP bridge in {:?}...", delay);
        tokio::select! {
            _ = sleep(delay) => {}
            _ = cancel.cancelled() => { return; }
        }
    }
}

/// Validate NMEA checksum: XOR of bytes between '$' and '*' must equal the
/// two hex digits after '*'.
fn nmea_checksum_ok(sentence: &str) -> bool {
    let Some(star) = sentence.rfind('*') else {
        return false;
    };
    let payload = &sentence[1..star];
    let declared = &sentence[star + 1..];
    if declared.len() < 2 {
        return false;
    }
    let Ok(expected) = u8::from_str_radix(&declared[..2], 16) else {
        return false;
    };
    let computed: u8 = payload.bytes().fold(0u8, |acc, b| acc ^ b);
    computed == expected
}

/// Read NMEA sentences from the TCP stream and forward events.
///
/// Returns `Ok(())` on graceful EOF or cancellation.
/// Returns `Err` on I/O errors or when the event channel is closed.
async fn read_from_stream(
    mut stream: TcpStream,
    event_tx: &mpsc::Sender<GpsEvent>,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut buffer = Vec::new();
    let mut temp_buf = [0u8; 8192];
    let mut sentences_received = 0u64;
    let mut last_log_time = tokio::time::Instant::now();

    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }

        if last_log_time.elapsed() > Duration::from_secs(30) {
            info!(
                "TCP bridge stats: {} NMEA sentences received in last 30s",
                sentences_received
            );
            sentences_received = 0;
            last_log_time = tokio::time::Instant::now();
        }

        let n = tokio::select! {
            result = stream.read(&mut temp_buf) => {
                match result {
                    // Graceful EOF — bridge closed the connection
                    Ok(0) => return Ok(()),
                    Ok(n) => n,
                    Err(e) => return Err(anyhow::anyhow!("TCP read error: {}", e)),
                }
            }
            _ = cancel.cancelled() => return Ok(()),
        };

        buffer.extend_from_slice(&temp_buf[..n]);

        // Discard any leading bytes before the first '$'
        if let Some(dollar) = buffer.iter().position(|&b| b == b'$') {
            if dollar > 0 {
                buffer.drain(..dollar);
            }
        } else {
            buffer.clear();
            continue;
        }

        // Process all complete NMEA lines in the buffer
        while let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = buffer.drain(..=newline_pos).collect();

            match String::from_utf8(line_bytes) {
                Ok(line) => {
                    let trimmed = line.trim();

                    if trimmed.starts_with('$')
                        && trimmed.contains('*')
                        && nmea_checksum_ok(trimmed)
                    {
                        sentences_received += 1;
                        NMEA_SENTENCES_TOTAL.inc();
                        debug!("TCP received: {}", trimmed);

                        if event_tx
                            .send(GpsEvent::RawNmea(trimmed.to_string()))
                            .await
                            .is_err()
                        {
                            return Err(anyhow::anyhow!("Event channel closed"));
                        }

                        match parse_nmea_sentence(trimmed) {
                            Ok(events) => {
                                for event in events {
                                    if event_tx.send(event).await.is_err() {
                                        return Err(anyhow::anyhow!("Event channel closed"));
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("Failed to parse TCP sentence: {}", e);
                            }
                        }
                    }
                }
                Err(_) => {
                    debug!("Skipping non-UTF-8 data from TCP stream");
                }
            }
        }

        // Safety net: discard if buffer exceeds 4 KB without a newline
        if buffer.len() > 4096 {
            warn!("TCP buffer overflow, resyncing to next NMEA sentence");
            if let Some(next) = buffer[1..].iter().position(|&b| b == b'$') {
                buffer.drain(..next + 1);
            } else {
                buffer.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nmea_checksum_ok_valid() {
        // $GPRMC with correct checksum
        assert!(nmea_checksum_ok(
            "$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A"
        ));
    }

    #[test]
    fn test_nmea_checksum_ok_invalid() {
        // Tampered checksum
        assert!(!nmea_checksum_ok(
            "$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*FF"
        ));
    }

    #[test]
    fn test_nmea_checksum_ok_no_star() {
        assert!(!nmea_checksum_ok("$GPRMC,123519,A,4807.038,N,01131.000,E"));
    }

    #[test]
    fn test_nmea_checksum_ok_short_checksum() {
        assert!(!nmea_checksum_ok("$GPRMC*6"));
    }

    #[test]
    fn test_nmea_checksum_ok_empty() {
        assert!(!nmea_checksum_ok(""));
    }
}
