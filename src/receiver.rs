//! Receiver profiles: vendor-specific configuration written after connecting.
//!
//! Only explicit settings cause writes. `receiver = "generic"` with no
//! `init_commands` never sends a byte to the device.

use crate::config::ReceiverProfile;
use crate::nmea::{MAX_LINE, with_checksum};
use anyhow::{Result, bail};

/// Build a UBX frame with its Fletcher-8 checksum.
pub fn ubx_frame(class: u8, id: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = vec![0xB5, 0x62, class, id, len as u8, (len >> 8) as u8];
    frame.extend_from_slice(payload);
    let (mut a, mut b) = (0u8, 0u8);
    for &byte in &frame[2..] {
        a = a.wrapping_add(byte);
        b = b.wrapping_add(a);
    }
    frame.extend_from_slice(&[a, b]);
    frame
}

/// UBX-CFG-RATE: measurement period in ms, one navigation solution per
/// measurement, aligned to GPS time.
pub fn ubx_cfg_rate(hz: u32) -> Vec<u8> {
    let period = (1000 / hz.max(1)) as u16;
    let [p0, p1] = period.to_le_bytes();
    ubx_frame(0x06, 0x08, &[p0, p1, 0x01, 0x00, 0x01, 0x00])
}

fn nmea_command(body: &str) -> Vec<u8> {
    format!("{}\r\n", with_checksum(body)).into_bytes()
}

/// Baud rate requested when the NMEA load does not fit the current link.
pub const HIGH_RATE_BAUD: u32 = 115_200;
/// Approximate bytes per epoch for RMC + GGA.
const CORE_BYTES_PER_EPOCH: u32 = 150;
/// Approximate bytes per second for the once-per-second GSA + multi-part GSV
/// burst (≈ 20 satellites, two constellations).
const SKY_BYTES_PER_SECOND: u32 = 700;

/// Whether `hz` epochs/s of RMC + GGA plus the per-second sky burst exceed
/// 80 % of a serial link's byte budget (10 bits per byte).
fn exceeds_link(hz: u32, baud: u32) -> bool {
    hz * CORE_BYTES_PER_EPOCH + SKY_BYTES_PER_SECOND > baud / 10 * 8 / 10
}

/// UBX-CFG-MSG (short form): output rate of an NMEA message on the current port,
/// in navigation epochs (0 = off).
pub fn ubx_cfg_msg(nmea_id: u8, rate: u8) -> Vec<u8> {
    ubx_frame(0x06, 0x01, &[0xF0, nmea_id, rate])
}

/// UBX-CFG-PRT for UART1: 8N1, UBX+NMEA+RTCM in, UBX+NMEA out.
///
/// On u-blox receivers with native USB the USB NMEA throughput is also bounded
/// by this setting (measured on an M8030: ~960 B/s at the 9600 default).
pub fn ubx_cfg_prt_uart1(baud: u32) -> Vec<u8> {
    let mut p = vec![0x01, 0x00, 0x00, 0x00];
    p.extend_from_slice(&0x0000_08D0u32.to_le_bytes());
    p.extend_from_slice(&baud.to_le_bytes());
    p.extend_from_slice(&[0x07, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00]);
    ubx_frame(0x06, 0x00, &p)
}

/// What to write after connecting, and whether to reopen the port at a new baud.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub commands: Vec<Vec<u8>>,
    pub reopen_baud: Option<u32>,
}

/// Build the write plan.
///
/// `link_baud` is the current serial baud when the link speed can be changed
/// (`None` for TCP bridges). For rates above 1 Hz the NMEA set is trimmed to
/// RMC + GGA every epoch with GSA/GSV once per second, and the port is raised to
/// [`HIGH_RATE_BAUD`] when RMC + GGA would not fit. Everything goes to receiver
/// RAM; a power cycle restores the stored configuration.
pub fn plan(
    profile: ReceiverProfile,
    rate_hz: Option<u32>,
    custom: &[String],
    link_baud: Option<u32>,
) -> Result<Plan> {
    let mut plan = Plan::default();
    for cmd in custom {
        plan.commands.push(encode_command(cmd)?);
    }
    let Some(hz) = rate_hz else {
        return Ok(plan);
    };
    let hz = hz.clamp(1, 25);
    let period_ms = 1000 / hz;
    // Only above 1 Hz: the factory sentence set at 1 Hz fits its default baud.
    let needs_baud = link_baud.filter(|b| hz > 1 && *b < HIGH_RATE_BAUD && exceeds_link(hz, *b));
    let per_second = hz.min(255) as u8;
    match profile {
        ReceiverProfile::Generic => {}
        ReceiverProfile::Ublox => {
            if hz > 1 {
                // GLL and VTG duplicate RMC; GSA/GSV once per second.
                plan.commands.extend([
                    ubx_cfg_msg(0x01, 0),
                    ubx_cfg_msg(0x05, 0),
                    ubx_cfg_msg(0x02, per_second),
                    ubx_cfg_msg(0x03, per_second),
                ]);
            }
            plan.commands.push(ubx_cfg_rate(hz));
            if let Some(_current) = needs_baud {
                // Must be last: a real UART switches speed immediately.
                plan.commands.push(ubx_cfg_prt_uart1(HIGH_RATE_BAUD));
                plan.reopen_baud = Some(HIGH_RATE_BAUD);
            }
        }
        ReceiverProfile::Mediatek => {
            if hz > 1 {
                // GLL,RMC,VTG,GGA,GSA,GSV,… as multiples of the fix interval (max 5).
                plan.commands.push(nmea_command(
                    "PMTK314,0,1,0,1,5,5,0,0,0,0,0,0,0,0,0,0,0,0,0",
                ));
            }
            plan.commands
                .push(nmea_command(&format!("PMTK220,{period_ms}")));
            if needs_baud.is_some() {
                plan.commands
                    .push(nmea_command(&format!("PMTK251,{HIGH_RATE_BAUD}")));
                plan.reopen_baud = Some(HIGH_RATE_BAUD);
            }
        }
        ReceiverProfile::Quectel => {
            plan.commands
                .push(nmea_command(&format!("PAIR050,{period_ms}")));
        }
    }
    Ok(plan)
}

/// Encode a user-supplied init command.
///
/// * `hex:B5 62 06 08 …` — raw bytes (spaces, commas or `0x` prefixes allowed)
/// * `$PMTK220,100*2F` — sent verbatim
/// * `PMTK220,100` or `$PMTK220,100` — `$`, checksum and CRLF are added
pub fn encode_command(cmd: &str) -> Result<Vec<u8>> {
    let cmd = cmd.trim();
    if let Some(hex) = cmd.strip_prefix("hex:") {
        let bytes = hex
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|t| !t.is_empty())
            .map(|t| u8::from_str_radix(t.trim_start_matches("0x"), 16))
            .collect::<Result<Vec<u8>, _>>()?;
        if bytes.is_empty() {
            bail!("empty hex command");
        }
        return Ok(bytes);
    }
    if cmd.is_empty() || cmd.len() > MAX_LINE || !cmd.bytes().all(|b| (0x20..0x7F).contains(&b)) {
        bail!("NMEA command must be 1–{MAX_LINE} printable ASCII characters");
    }
    let body = cmd.strip_prefix('$').unwrap_or(cmd);
    if body.contains('*') {
        Ok(format!("${body}\r\n").into_bytes())
    } else {
        Ok(nmea_command(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ubx_10hz_matches_known_frame() {
        // Byte-identical to the frame shipped in 0.6.0.
        assert_eq!(
            ubx_cfg_rate(10),
            [
                0xB5, 0x62, 0x06, 0x08, 0x06, 0x00, 0x64, 0x00, 0x01, 0x00, 0x01, 0x00, 0x7A, 0x12
            ]
        );
        assert_eq!(&ubx_cfg_rate(5)[6..8], &[0xC8, 0x00]);
    }

    #[test]
    fn ublox_plans() {
        // 1 Hz: rate only, no baud change.
        let p = plan(ReceiverProfile::Ublox, Some(1), &[], Some(9600)).unwrap();
        assert_eq!(p.commands, [ubx_cfg_rate(1)]);
        assert_eq!(p.reopen_baud, None);
        // 10 Hz on a 9600 serial link: trim, rate, then raise baud last.
        let p = plan(ReceiverProfile::Ublox, Some(10), &[], Some(9600)).unwrap();
        assert_eq!(p.commands.len(), 6);
        assert_eq!(p.commands[0], ubx_cfg_msg(0x01, 0));
        assert_eq!(p.commands[4], ubx_cfg_rate(10));
        assert_eq!(p.commands[5], ubx_cfg_prt_uart1(115_200));
        assert_eq!(p.reopen_baud, Some(115_200));
        // 5 Hz still overflows 9600 once the per-second GSV burst is counted
        // (measured: only the first GSV part survived on an M8030).
        assert_eq!(
            plan(ReceiverProfile::Ublox, Some(5), &[], Some(9600))
                .unwrap()
                .reopen_baud,
            Some(115_200)
        );
        assert!(!exceeds_link(10, 115_200));
        // Already fast, or over TCP: no baud change.
        assert_eq!(
            plan(ReceiverProfile::Ublox, Some(10), &[], Some(115_200))
                .unwrap()
                .reopen_baud,
            None
        );
        assert_eq!(
            plan(ReceiverProfile::Ublox, Some(10), &[], None)
                .unwrap()
                .reopen_baud,
            None
        );
    }

    #[test]
    fn ubx_frames_match_receiver_verified_bytes() {
        // Frames verified against a u-blox M8030 (protocol 18).
        assert_eq!(
            ubx_cfg_msg(0x01, 0),
            [
                0xB5, 0x62, 0x06, 0x01, 0x03, 0x00, 0xF0, 0x01, 0x00, 0xFB, 0x11
            ]
        );
        assert_eq!(
            ubx_cfg_prt_uart1(115_200),
            [
                0xB5, 0x62, 0x06, 0x00, 0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0xD0, 0x08, 0x00, 0x00,
                0x00, 0xC2, 0x01, 0x00, 0x07, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x7E
            ]
        );
    }

    #[test]
    fn vendor_plans() {
        let p = plan(ReceiverProfile::Mediatek, Some(10), &[], Some(9600)).unwrap();
        assert_eq!(p.commands[1], b"$PMTK220,100*2F\r\n".to_vec());
        assert_eq!(p.commands[2], nmea_command("PMTK251,115200"));
        assert_eq!(p.reopen_baud, Some(115_200));
        let p = plan(ReceiverProfile::Quectel, Some(5), &[], Some(115_200)).unwrap();
        assert_eq!(p.commands, [nmea_command("PAIR050,200")]);
        assert_eq!(
            plan(ReceiverProfile::Generic, Some(10), &[], Some(9600)).unwrap(),
            Plan::default()
        );
    }

    #[test]
    fn custom_commands() {
        assert_eq!(
            encode_command("PMTK220,100").unwrap(),
            b"$PMTK220,100*2F\r\n"
        );
        assert_eq!(
            encode_command("$PMTK220,100").unwrap(),
            b"$PMTK220,100*2F\r\n"
        );
        assert_eq!(
            encode_command("$PMTK220,100*2F").unwrap(),
            b"$PMTK220,100*2F\r\n"
        );
        assert_eq!(
            encode_command("hex:B5 62,0x06").unwrap(),
            [0xB5, 0x62, 0x06]
        );
        for bad in ["", "hex:", "hex:GG", "PMTK\u{7}"] {
            assert!(encode_command(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn custom_commands_come_first() {
        let p = plan(
            ReceiverProfile::Ublox,
            Some(10),
            &["PUBX,40,GLL,0,0,0,0".into()],
            None,
        )
        .unwrap();
        assert_eq!(p.commands[0][0], b'$');
        assert_eq!(p.commands.last().unwrap(), &ubx_cfg_rate(10));
        assert!(
            plan(ReceiverProfile::Generic, None, &[], None)
                .unwrap()
                .commands
                .is_empty()
        );
    }
}
