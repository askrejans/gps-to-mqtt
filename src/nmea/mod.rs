//! NMEA-0183 framing, checksum verification and field access.
//!
//! Everything that touches untrusted bytes from a receiver or a network peer
//! lives here: [`LineFramer`] turns an arbitrary byte stream into bounded ASCII
//! lines, and [`Sentence::parse`] validates structure and checksum before any
//! field is interpreted.

pub mod fields;

use std::fmt;

/// Longest line accepted from a source. The NMEA spec limit is 82 bytes;
/// proprietary sentences (PUBX, PAIR, PMTK …) and NMEA 4.11 GSV can be longer.
pub const MAX_LINE: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NmeaError {
    #[error("sentence does not start with '$' or '!'")]
    NoStart,
    #[error("sentence has no checksum")]
    NoChecksum,
    #[error("malformed checksum")]
    BadChecksumFormat,
    #[error("checksum mismatch (declared {declared:02X}, computed {computed:02X})")]
    ChecksumMismatch { declared: u8, computed: u8 },
    #[error("sentence address field is malformed")]
    BadAddress,
}

/// XOR checksum of the bytes between the start delimiter and `*`.
pub fn checksum(body: &str) -> u8 {
    body.bytes().fold(0, |acc, b| acc ^ b)
}

/// Build a complete sentence (`$BODY*CS`) from a body without delimiters.
pub fn with_checksum(body: &str) -> String {
    format!("${body}*{:02X}", checksum(body))
}

/// A checksum-verified NMEA sentence borrowed from its source line.
#[derive(Clone, PartialEq)]
pub struct Sentence<'a> {
    /// Full original line (without line terminator).
    pub raw: &'a str,
    /// Two-letter talker (`GP`, `GN`, `GL` …) or `P` for proprietary sentences.
    pub talker: &'a str,
    /// Sentence formatter (`RMC`, `GGA` …) or proprietary identifier (`UBX`, `MTK001`).
    pub kind: &'a str,
    fields: Vec<&'a str>,
}

impl fmt::Debug for Sentence<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.raw)
    }
}

impl<'a> Sentence<'a> {
    /// Parse and verify a line. `require_checksum = false` accepts sentences
    /// without `*hh` (some cheap receivers omit it); a checksum that is present
    /// is always verified.
    pub fn parse(line: &'a str, require_checksum: bool) -> Result<Self, NmeaError> {
        let raw = line.trim_end_matches(['\r', '\n']);
        if !raw.is_ascii() {
            return Err(NmeaError::NoStart);
        }
        let rest = raw
            .strip_prefix('$')
            .or_else(|| raw.strip_prefix('!'))
            .ok_or(NmeaError::NoStart)?;

        let body = match rest.rsplit_once('*') {
            Some((body, cs)) => {
                let declared = (cs.len() == 2)
                    .then(|| u8::from_str_radix(cs, 16).ok())
                    .flatten()
                    .ok_or(NmeaError::BadChecksumFormat)?;
                let computed = checksum(body);
                if declared != computed {
                    return Err(NmeaError::ChecksumMismatch { declared, computed });
                }
                body
            }
            None if require_checksum => return Err(NmeaError::NoChecksum),
            None => rest,
        };

        let mut parts = body.split(',');
        let address = parts.next().unwrap_or_default();
        if address.is_empty() || !address.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(NmeaError::BadAddress);
        }
        let (talker, kind) = if let Some(kind) = address.strip_prefix('P') {
            ("P", kind)
        } else if address.len() == 5 {
            address.split_at(2)
        } else {
            return Err(NmeaError::BadAddress);
        };

        Ok(Self {
            raw,
            talker,
            kind,
            fields: parts.collect(),
        })
    }

    /// Data field `i` (0-based, address excluded). Missing fields read as empty.
    pub fn field(&self, i: usize) -> &'a str {
        self.fields.get(i).copied().unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }
}

/// Splits an arbitrary byte stream into bounded, printable-ASCII lines.
///
/// Binary protocol frames (UBX, RTCM) interleaved with NMEA, partial lines
/// after a reconnect and over-long garbage are discarded without allocating
/// more than [`MAX_LINE`] bytes.
#[derive(Default)]
pub struct LineFramer {
    buf: Vec<u8>,
    in_line: bool,
    overflowed: bool,
}

impl LineFramer {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(MAX_LINE),
            ..Self::default()
        }
    }

    /// Feed bytes, calling `emit` for every complete candidate line.
    pub fn push(&mut self, bytes: &[u8], mut emit: impl FnMut(&str)) {
        for &b in bytes {
            match b {
                b'$' | b'!' => {
                    // A new start delimiter always resynchronises.
                    self.buf.clear();
                    self.buf.push(b);
                    self.in_line = true;
                    self.overflowed = false;
                }
                b'\r' | b'\n' => {
                    if self.in_line && !self.overflowed && self.buf.len() > 1 {
                        // Only printable ASCII is buffered, so this cannot fail.
                        if let Ok(line) = std::str::from_utf8(&self.buf) {
                            emit(line);
                        }
                    }
                    self.buf.clear();
                    self.in_line = false;
                }
                0x20..=0x7E if self.in_line => {
                    if self.buf.len() >= MAX_LINE {
                        self.overflowed = true;
                    } else {
                        self.buf.push(b);
                    }
                }
                _ => {
                    // Non-printable byte inside a line: binary frame, drop the line.
                    self.buf.clear();
                    self.in_line = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_sentence() {
        let s = Sentence::parse(
            "$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A",
            true,
        )
        .unwrap();
        assert_eq!(s.talker, "GP");
        assert_eq!(s.kind, "RMC");
        assert_eq!(s.field(0), "123519");
        assert_eq!(s.field(8), "230394");
        assert_eq!(s.field(99), "");
    }

    #[test]
    fn parses_proprietary_sentence() {
        let line = with_checksum("PUBX,00,081350.00,4717.113210,N");
        let s = Sentence::parse(&line, true).unwrap();
        assert_eq!(s.talker, "P");
        assert_eq!(s.kind, "UBX");
        assert_eq!(s.field(0), "00");
    }

    #[test]
    fn rejects_bad_checksums_without_panicking() {
        let base = "$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W";
        assert!(matches!(
            Sentence::parse(&format!("{base}*FF"), true),
            Err(NmeaError::ChecksumMismatch { .. })
        ));
        assert_eq!(Sentence::parse(base, true), Err(NmeaError::NoChecksum));
        assert!(Sentence::parse(base, false).is_ok());
        for bad in ["$GPRMC*6", "$GPRMC*6AA", "$*00", "", "$", "$GP€*00", "$G*€"] {
            assert!(Sentence::parse(bad, true).is_err(), "{bad}");
        }
    }

    #[test]
    fn talker_does_not_leak_into_kind_detection() {
        // A TXT sentence whose payload mentions GGA must still be a TXT.
        let line = with_checksum("GPTXT,01,01,02,GGA RMC enabled");
        let s = Sentence::parse(&line, true).unwrap();
        assert_eq!(s.kind, "TXT");
    }

    #[test]
    fn framer_splits_lines_and_drops_binary() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        let ubx = [0xB5, 0x62, 0x01, 0x07, b'$', 0x00, 0xFF];
        f.push(b"garbage$GPGGA,1*00\r\n$GPR", |l| out.push(l.to_string()));
        f.push(&ubx, |l| out.push(l.to_string()));
        f.push(b"$GPRMC,2*00\r\n", |l| out.push(l.to_string()));
        assert_eq!(out, ["$GPGGA,1*00", "$GPRMC,2*00"]);
    }

    #[test]
    fn framer_bounds_memory_on_endless_line() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        f.push(b"$", |_| {});
        for _ in 0..10_000 {
            f.push(b"AAAAAAAAAA", |l| out.push(l.to_string()));
        }
        f.push(b"\n$GPX,1\n", |l| out.push(l.to_string()));
        assert!(f.buf.capacity() <= MAX_LINE * 2);
        assert_eq!(out, ["$GPX,1"]);
    }
}
