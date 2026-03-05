use crate::config::AppConfig;
use crate::parser::{GpsEvent, parse_nmea_sentence};
use anyhow::{Context, Result};
use serialport::SerialPort;
use std::io::Read;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_CONSECUTIVE_ERRORS: u32 = 3;
const ERROR_BACKOFF_DELAY: Duration = Duration::from_secs(10);

/// UBX command to set GPS to 10Hz update rate
const UBX_10HZ_COMMAND: &[u8] = &[
    0xB5, 0x62, 0x06, 0x08, 0x06, 0x00, 0x64, 0x00, 0x01, 0x00, 0x01, 0x00, 0x7A, 0x12,
];

/// Spawn the serial port reading task
pub async fn spawn_serial_task(
    config: AppConfig,
    event_tx: mpsc::Sender<GpsEvent>,
    cancel: CancellationToken,
) -> Result<()> {
    let port_name = config.port_name.clone();
    let baud_rate = config.baud_rate;
    let set_10hz = config.set_gps_to_10hz;

    tokio::task::spawn_blocking(move || {
        run_serial_loop(&port_name, baud_rate, set_10hz, event_tx, cancel)
    });

    Ok(())
}

/// Run the serial port reading loop with reconnection
fn run_serial_loop(
    port_name: &str,
    baud_rate: u32,
    set_10hz: bool,
    event_tx: mpsc::Sender<GpsEvent>,
    cancel: CancellationToken,
) {
    let mut consecutive_errors = 0;

    loop {
        if cancel.is_cancelled() {
            info!("Serial loop shutting down");
            break;
        }

        info!("Opening serial port: {} at {} baud", port_name, baud_rate);

        match open_serial_port(port_name, baud_rate) {
            Ok(mut port) => {
                info!("Serial port opened successfully");
                consecutive_errors = 0;

                // Configure GPS to 10Hz if requested
                if set_10hz {
                    if let Err(e) = configure_10hz(&mut port) {
                        warn!("Failed to configure 10Hz mode: {}", e);
                    }
                }

                // Read from port
                match read_from_port(&mut port, &event_tx, &cancel) {
                    Ok(_) => {
                        info!("Serial port closed gracefully");
                        break;
                    }
                    Err(e) => {
                        // Channel closed means the app is shutting down — don't reconnect
                        if e.to_string().contains("channel closed") || cancel.is_cancelled() {
                            info!("Serial loop exiting ({})", e);
                            break;
                        }
                        error!("Serial read error: {}", e);
                        consecutive_errors += 1;
                    }
                }
            }
            Err(e) => {
                if cancel.is_cancelled() {
                    break;
                }
                error!("Failed to open serial port: {}", e);
                consecutive_errors += 1;
            }
        }

        if cancel.is_cancelled() {
            break;
        }

        // Determine reconnection delay based on error count
        let delay = if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
            warn!("Max consecutive errors reached, using extended backoff delay");
            ERROR_BACKOFF_DELAY
        } else {
            RECONNECT_DELAY
        };

        warn!("Reconnecting in {:?}...", delay);
        // Sleep in short chunks so we can respond to cancellation promptly
        let deadline = std::time::Instant::now() + delay;
        while std::time::Instant::now() < deadline {
            if cancel.is_cancelled() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Open a serial port with the specified configuration
fn open_serial_port(port_name: &str, baud_rate: u32) -> Result<Box<dyn SerialPort>> {
    let port = serialport::new(port_name, baud_rate)
        .timeout(Duration::from_millis(500)) // Balanced timeout for stability
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .open()
        .context("Failed to open serial port")?;

    Ok(port)
}

/// Configure GPS to 10Hz update rate (u-blox specific)
fn configure_10hz(port: &mut Box<dyn SerialPort>) -> Result<()> {
    info!("Configuring GPS for 10Hz update rate");

    use std::io::Write;
    port.write_all(UBX_10HZ_COMMAND)
        .context("Failed to write 10Hz command")?;

    port.flush().context("Failed to flush serial port")?;

    info!("10Hz configuration sent");
    Ok(())
}

/// Validate NMEA checksum: XOR of bytes between '$' and '*' must equal the two hex digits after '*'.
fn nmea_checksum_ok(sentence: &str) -> bool {
    let Some(star) = sentence.rfind('*') else {
        return false;
    };
    let payload = &sentence[1..star]; // between '$' and '*'
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

/// Read NMEA sentences from the serial port and send events
fn read_from_port(
    port: &mut Box<dyn SerialPort>,
    event_tx: &mpsc::Sender<GpsEvent>,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut buffer = Vec::new();
    // 8 KB — large enough to drain the UART FIFO quickly at 115200 baud (≈11.5 KB/s)
    let mut temp_buf = [0u8; 8192];
    let mut timeout_count = 0;
    let mut sentences_received = 0;
    let mut last_log_time = std::time::Instant::now();
    const MAX_CONSECUTIVE_TIMEOUTS: u32 = 100;

    loop {
        // Exit cleanly when cancelled
        if cancel.is_cancelled() {
            return Ok(());
        }

        // Log statistics every 30 seconds
        if last_log_time.elapsed() > Duration::from_secs(30) {
            info!(
                "Serial port stats: {} NMEA sentences received in last 30s",
                sentences_received
            );
            sentences_received = 0;
            last_log_time = std::time::Instant::now();
        }

        // Read raw bytes
        match port.read(&mut temp_buf) {
            Ok(0) => {
                // No data available - this is normal, just wait a bit
                std::thread::sleep(Duration::from_millis(10));
                timeout_count += 1;
                if timeout_count > MAX_CONSECUTIVE_TIMEOUTS {
                    warn!("No data received for extended period");
                    timeout_count = 0; // Reset but don't fail
                }
                continue;
            }
            Ok(n) => {
                timeout_count = 0;

                buffer.extend_from_slice(&temp_buf[..n]);

                // Discard any leading bytes before the first '$' (binary UBX frames,
                // partial sentences from reconnect, etc.) so they never accumulate.
                if let Some(dollar) = buffer.iter().position(|&b| b == b'$') {
                    if dollar > 0 {
                        buffer.drain(..dollar);
                    }
                } else {
                    // No NMEA start at all — toss everything and wait for next read
                    buffer.clear();
                    continue;
                }

                // Process complete lines from buffer
                while let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
                    // Extract line (including the newline)
                    let line_bytes: Vec<u8> = buffer.drain(..=newline_pos).collect();

                    // Try to convert to UTF-8, skip if invalid
                    match String::from_utf8(line_bytes) {
                        Ok(line) => {
                            let trimmed = line.trim();

                            // Only process fully-valid NMEA sentences (checksum guards
                            // against garbled data immediately after reconnect)
                            if trimmed.starts_with('$')
                                && trimmed.contains('*')
                                && nmea_checksum_ok(trimmed)
                            {
                                sentences_received += 1;
                                debug!("Received: {}", trimmed);

                                // Send raw NMEA sentence first
                                let raw_event = GpsEvent::RawNmea(trimmed.to_string());
                                if let Err(_) = event_tx.blocking_send(raw_event) {
                                    return Err(anyhow::anyhow!("Event channel closed"));
                                }

                                // Parse the sentence
                                match parse_nmea_sentence(trimmed) {
                                    Ok(events) => {
                                        for event in events {
                                            if let Err(_) = event_tx.blocking_send(event) {
                                                return Err(anyhow::anyhow!(
                                                    "Event channel closed"
                                                ));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        debug!("Failed to parse sentence: {}", e);
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // Skip invalid UTF-8 data (likely binary UBX messages or noise)
                            debug!("Skipping non-UTF-8 data");
                        }
                    }
                }

                // Safety net: a single NMEA sentence is never longer than 82 bytes
                // (spec limit 80 + CRLF).  If the buffer somehow grew past 4 KB without
                // a newline the stream is hopelessly out of sync — discard up to next '$'.
                if buffer.len() > 4096 {
                    warn!("Serial buffer overflow, resyncing to next NMEA sentence");
                    // skip the current '$' (index 0) and find the next one
                    if let Some(next) = buffer[1..].iter().position(|&b| b == b'$') {
                        buffer.drain(..next + 1);
                    } else {
                        buffer.clear();
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                // Timeout is normal with short timeout, just continue
                timeout_count += 1;
                if timeout_count > MAX_CONSECUTIVE_TIMEOUTS {
                    debug!("Many consecutive timeouts, but continuing...");
                    timeout_count = 0;
                }
                continue;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Would block is like timeout - no data available
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                // Interrupted system call - retry
                continue;
            }
            Err(e) => {
                // Real error - return it
                return Err(anyhow::anyhow!("Error reading from serial port: {}", e));
            }
        }
    }
}
