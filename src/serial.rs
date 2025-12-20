use crate::config::AppConfig;
use crate::parser::{parse_nmea_sentence, GpsEvent};
use anyhow::{Context, Result};
use serialport::SerialPort;
use std::io::Read;
use std::time::Duration;
use tokio::sync::mpsc;
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
) -> Result<()> {
    let port_name = config.port_name.clone();
    let baud_rate = config.baud_rate;
    let set_10hz = config.set_gps_to_10hz;

    tokio::task::spawn_blocking(move || {
        run_serial_loop(&port_name, baud_rate, set_10hz, event_tx)
    });

    Ok(())
}

/// Run the serial port reading loop with reconnection
fn run_serial_loop(
    port_name: &str,
    baud_rate: u32,
    set_10hz: bool,
    event_tx: mpsc::Sender<GpsEvent>,
) {
    let mut consecutive_errors = 0;

    loop {
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
                if let Err(e) = read_from_port(&mut port, &event_tx) {
                    error!("Error reading from serial port: {}", e);
                    consecutive_errors += 1;
                } else {
                    info!("Serial port closed gracefully");
                    break;
                }
            }
            Err(e) => {
                error!("Failed to open serial port: {}", e);
                consecutive_errors += 1;
            }
        }

        // Determine reconnection delay based on error count
        let delay = if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
            warn!(
                "Max consecutive errors reached, using extended backoff delay"
            );
            ERROR_BACKOFF_DELAY
        } else {
            RECONNECT_DELAY
        };

        warn!("Reconnecting in {:?}...", delay);
        std::thread::sleep(delay);
    }
}

/// Open a serial port with the specified configuration
fn open_serial_port(port_name: &str, baud_rate: u32) -> Result<Box<dyn SerialPort>> {
    let port = serialport::new(port_name, baud_rate)
        .timeout(Duration::from_secs(5))
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

/// Read NMEA sentences from the serial port and send events
fn read_from_port(
    port: &mut Box<dyn SerialPort>,
    event_tx: &mpsc::Sender<GpsEvent>,
) -> Result<()> {
    let mut buffer = Vec::new();
    let mut temp_buf = [0u8; 512];

    loop {
        // Read raw bytes instead of using BufReader with read_line
        match port.read(&mut temp_buf) {
            Ok(0) => {
                // EOF - this shouldn't happen with serial ports
                warn!("Unexpected end of serial port stream");
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Ok(n) => {
                // Append received bytes to buffer
                buffer.extend_from_slice(&temp_buf[..n]);
                
                // Process complete lines from buffer
                while let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
                    // Extract line (including the newline)
                    let line_bytes: Vec<u8> = buffer.drain(..=newline_pos).collect();
                    
                    // Try to convert to UTF-8, skip if invalid
                    match String::from_utf8(line_bytes) {
                        Ok(line) => {
                            let trimmed = line.trim();
                            
                            // Only process NMEA sentences
                            if trimmed.starts_with('$') && trimmed.contains('*') {
                                debug!("Received: {}", trimmed);
                                
                                // Parse the sentence
                                match parse_nmea_sentence(trimmed) {
                                    Ok(events) => {
                                        for event in events {
                                            // Send events to the processing task
                                            if let Err(e) = event_tx.blocking_send(event) {
                                                error!("Failed to send GPS event: {}", e);
                                                return Err(anyhow::anyhow!("Event channel closed"));
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
                
                // Keep buffer size reasonable - if it gets too large without finding a newline,
                // something is wrong, so discard old data
                if buffer.len() > 4096 {
                    warn!("Serial buffer overflow, clearing");
                    buffer.clear();
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                // Timeout is normal, continue reading
                continue;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                // Broken pipe might be recoverable
                warn!("Broken pipe detected on serial port");
                return Err(anyhow::anyhow!("Broken pipe"));
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Error reading from serial port: {}", e));
            }
        }
    }
}
