//! Serial source with receiver auto-detection.
//!
//! `port_name = "auto"` ranks serial ports (known GNSS USB vendors first),
//! `baud_rate = 0` probes common rates; a port/baud is accepted only after it
//! yields checksum-valid NMEA. Detection repeats after every disconnect, so a
//! receiver that re-enumerates under a different path is found again.

use super::{Backoff, Tx, status};
use crate::config::AppConfig;
use crate::hub::SourceEvent;
use crate::nmea::{LineFramer, Sentence};
use crate::receiver;
use crate::shared::LinkStatus;
use anyhow::{Context, Result, bail};
use serialport::{SerialPort, SerialPortInfo, SerialPortType};
use std::io::ErrorKind;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Probe order: the factory defaults of most receivers first.
pub const PROBE_BAUDS: &[u32] = &[
    9600, 115200, 38400, 4800, 57600, 230400, 460800, 921600, 19200,
];
const PROBE_TIME: Duration = Duration::from_millis(1600);
const READ_TIMEOUT: Duration = Duration::from_millis(250);

/// USB vendors that ship GNSS receivers or the USB-UART bridges used on them.
const KNOWN_VENDORS: &[(u16, &str)] = &[
    (0x1546, "u-blox"),
    (0x0E8D, "MediaTek"),
    (0x2C7C, "Quectel"),
    (0x091E, "Garmin"),
    (0x1199, "Sierra Wireless"),
    (0x067B, "Prolific PL2303"),
    (0x10C4, "Silicon Labs CP210x"),
    (0x1A86, "WCH CH34x"),
    (0x0403, "FTDI"),
];

#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: String,
    pub description: String,
    pub score: u8,
}

fn describe(info: &SerialPortInfo) -> (String, u8) {
    match &info.port_type {
        SerialPortType::UsbPort(usb) => {
            let vendor = KNOWN_VENDORS.iter().find(|(v, _)| *v == usb.vid);
            let product = usb
                .product
                .clone()
                .or_else(|| usb.manufacturer.clone())
                .unwrap_or_default();
            let label = format!(
                "USB {:04x}:{:04x} {}{}",
                usb.vid,
                usb.pid,
                vendor.map(|(_, n)| format!("[{n}] ")).unwrap_or_default(),
                product
            );
            let gnss_name = product.to_lowercase();
            let score = match vendor {
                Some((0x1546 | 0x0E8D | 0x2C7C | 0x091E, _)) => 100,
                _ if gnss_name.contains("gps") || gnss_name.contains("gnss") => 95,
                Some(_) => 80,
                None => 60,
            };
            (label.trim_end().to_string(), score)
        }
        SerialPortType::PciPort => ("PCI serial".into(), 20),
        SerialPortType::BluetoothPort => ("Bluetooth".into(), 10),
        SerialPortType::Unknown => ("serial".into(), 30),
    }
}

/// Rank serial ports most-likely-GNSS first; skips consoles and macOS `tty.*` duplicates.
pub fn candidates() -> Vec<Candidate> {
    let ports = serialport::available_ports().unwrap_or_default();
    let mut out: Vec<Candidate> = ports
        .iter()
        .filter(|p| {
            let n = p.port_name.as_str();
            !(n.starts_with("/dev/tty.")
                || n.contains("Bluetooth")
                || n.contains("debug-console")
                || n.contains("wlan-debug"))
        })
        .map(|p| {
            let (description, mut score) = describe(p);
            let n = p.port_name.as_str();
            // Raspberry Pi GNSS HATs live on the primary UART.
            if n.ends_with("serial0") || n.contains("ttyAMA") {
                score = score.max(50);
            }
            // Plain motherboard UARTs: probing dozens of them is slow and rarely useful.
            if n.starts_with("/dev/ttyS") && score <= 30 {
                score = 5;
            }
            Candidate {
                path: p.port_name.clone(),
                description,
                score,
            }
        })
        .collect();
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.path.cmp(&b.path)));
    out
}

fn open(path: &str, baud: u32) -> Result<Box<dyn SerialPort>> {
    serialport::new(path, baud)
        .timeout(READ_TIMEOUT)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .open()
        .with_context(|| format!("opening {path} at {baud} baud"))
}

/// Count checksum-valid sentences read within `PROBE_TIME`; returns the first
/// receiver identification text if seen.
fn probe(port: &mut dyn SerialPort, cancel: &CancellationToken) -> (usize, Option<String>) {
    let mut framer = LineFramer::new();
    let mut buf = [0u8; 1024];
    let mut valid = 0;
    let mut ident = None;
    let deadline = Instant::now() + PROBE_TIME;
    while Instant::now() < deadline && valid < 3 && !cancel.is_cancelled() {
        match port.read(&mut buf) {
            Ok(n) => framer.push(&buf[..n], |line| {
                if let Ok(s) = Sentence::parse(line, true) {
                    valid += 1;
                    if s.kind == "TXT" && ident.is_none() {
                        ident = Some(s.field(3).to_string());
                    }
                }
            }),
            Err(e) if e.kind() == ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
    (valid, ident)
}

/// Resolve `(path, baud)` according to config, probing where requested.
pub fn detect(
    config: &AppConfig,
    cancel: &CancellationToken,
) -> Result<(String, u32, Box<dyn SerialPort>)> {
    let paths: Vec<Candidate> = if config.port_name.eq_ignore_ascii_case("auto") {
        let c = candidates();
        if c.is_empty() {
            bail!("no serial ports found");
        }
        c
    } else {
        vec![Candidate {
            path: config.port_name.clone(),
            description: String::new(),
            score: 0,
        }]
    };
    let bauds: Vec<u32> = if config.baud_rate == 0 {
        PROBE_BAUDS.to_vec()
    } else {
        vec![config.baud_rate]
    };
    let must_probe = paths.len() > 1 || bauds.len() > 1;

    for candidate in &paths {
        for &baud in &bauds {
            if cancel.is_cancelled() {
                bail!("cancelled");
            }
            let mut port = match open(&candidate.path, baud) {
                Ok(p) => p,
                Err(e) => {
                    debug!("{e:#}");
                    break; // next path: a port that cannot open won't open at another baud
                }
            };
            if !must_probe {
                return Ok((candidate.path.clone(), baud, port));
            }
            let (valid, ident) = probe(port.as_mut(), cancel);
            if valid >= 2 {
                info!(
                    "Detected NMEA receiver on {} @ {} baud {}{}",
                    candidate.path,
                    baud,
                    candidate.description,
                    ident.map(|i| format!(" — {i}")).unwrap_or_default()
                );
                return Ok((candidate.path.clone(), baud, port));
            }
            debug!("{} @ {}: no NMEA", candidate.path, baud);
        }
    }
    bail!(
        "no NMEA receiver found on {} port(s) at {:?} baud",
        paths.len(),
        bauds
    )
}

/// Write the receiver plan. Returns the port to read from — possibly reopened
/// at a higher baud — and its baud rate; `None` if the port could not be
/// reopened (the caller reconnects).
fn apply_plan(
    mut port: Box<dyn SerialPort>,
    path: &str,
    baud: u32,
    config: &AppConfig,
    cancel: &CancellationToken,
) -> Option<(Box<dyn SerialPort>, u32)> {
    let plan = match receiver::plan(
        config.effective_receiver(),
        config.effective_update_rate(),
        &config.init_commands,
        Some(baud),
    ) {
        Ok(plan) => plan,
        Err(e) => {
            warn!("Receiver configuration skipped: {e:#}");
            return Some((port, baud));
        }
    };
    for cmd in &plan.commands {
        if let Err(e) = port.write_all(cmd).and_then(|_| port.flush()) {
            warn!("Failed to write receiver command: {e}");
            return Some((port, baud));
        }
        // Receivers drop commands that arrive back-to-back with their output.
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(hz) = config.effective_update_rate() {
        info!(
            "Configured {hz} Hz navigation rate ({:?})",
            config.effective_receiver()
        );
    }
    let Some(new_baud) = plan.reopen_baud else {
        return Some((port, baud));
    };
    std::thread::sleep(Duration::from_millis(150));
    drop(port);
    if let Ok(mut fast) = open(path, new_baud)
        && probe(fast.as_mut(), cancel).0 >= 2
    {
        info!(
            "Receiver switched to {new_baud} baud for {} Hz output",
            config.effective_update_rate().unwrap_or(1)
        );
        return Some((fast, new_baud));
    }
    warn!("No NMEA at {new_baud} baud after reconfiguration; staying at {baud}");
    open(path, baud)
        .map_err(|e| warn!("{e:#}"))
        .ok()
        .map(|p| (p, baud))
}

/// Blocking serial loop (runs on a blocking thread).
pub fn run(config: &AppConfig, tx: Tx, cancel: CancellationToken) {
    let mut backoff = Backoff::new();
    let handle = tokio::runtime::Handle::current();
    while !cancel.is_cancelled() {
        status(&tx, LinkStatus::Connecting, config.connection_display());
        match detect(config, &cancel) {
            Ok((path, baud, port)) => {
                backoff.reset();
                if let Some((mut port, baud)) = apply_plan(port, &path, baud, config, &cancel) {
                    let detail = format!("{path} @ {baud} baud");
                    info!("Serial source open: {detail}");
                    status(&tx, LinkStatus::Connected, detail);
                    match read_loop(port.as_mut(), &tx, &cancel) {
                        Ok(()) => return,
                        Err(e) => error!("Serial read error: {e:#}"),
                    }
                }
            }
            Err(e) if cancel.is_cancelled() => {
                debug!("{e:#}");
                return;
            }
            Err(e) => warn!("Serial source unavailable: {e:#}"),
        }
        status(&tx, LinkStatus::Disconnected, "");
        if !handle.block_on(super::wait(&mut backoff, &cancel)) {
            return;
        }
    }
}

fn read_loop(port: &mut dyn SerialPort, tx: &Tx, cancel: &CancellationToken) -> Result<()> {
    let mut buf = [0u8; 4096];
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }
        match port.read(&mut buf) {
            Ok(0) => std::thread::sleep(Duration::from_millis(10)),
            Ok(n) => {
                if tx
                    .blocking_send(SourceEvent::Bytes(buf[..n].to_vec()))
                    .is_err()
                {
                    return Ok(()); // hub gone: shutting down
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e.into()),
        }
    }
}

/// `--list-ports`: print ranked ports, optionally probing each one.
pub fn list_ports(probe_ports: bool) {
    let list = candidates();
    if list.is_empty() {
        println!("No serial ports found.");
        return;
    }
    let cancel = CancellationToken::new();
    for c in list {
        print!("{:<32} score {:>3}  {}", c.path, c.score, c.description);
        if probe_ports {
            let mut found = None;
            for &baud in PROBE_BAUDS {
                if let Ok(mut port) = open(&c.path, baud) {
                    let (valid, ident) = probe(port.as_mut(), &cancel);
                    if valid >= 2 {
                        found = Some((baud, ident));
                        break;
                    }
                } else {
                    break;
                }
            }
            match found {
                Some((baud, ident)) => print!(
                    "  → NMEA @ {baud}{}",
                    ident.map(|i| format!(" ({i})")).unwrap_or_default()
                ),
                None => print!("  → no NMEA"),
            }
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::UsbPortInfo;

    fn usb(vid: u16, product: &str) -> SerialPortInfo {
        SerialPortInfo {
            port_name: "/dev/x".into(),
            port_type: SerialPortType::UsbPort(UsbPortInfo {
                vid,
                pid: 0x01a8,
                serial_number: None,
                manufacturer: None,
                product: Some(product.into()),
            }),
        }
    }

    #[test]
    fn known_gnss_vendors_rank_first() {
        let (label, score) = describe(&usb(0x1546, "u-blox GNSS receiver"));
        assert_eq!(score, 100);
        assert!(label.contains("1546:01a8 [u-blox]"));
        assert_eq!(describe(&usb(0x067B, "USB-Serial")).1, 80);
        assert_eq!(describe(&usb(0x1234, "GPS Dongle")).1, 95);
        assert_eq!(describe(&usb(0x1234, "Keyboard")).1, 60);
    }
}
