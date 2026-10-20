//! Strict parsers for individual NMEA field formats.

use anyhow::{Context, Result, bail};
use chrono::{NaiveDate, NaiveTime};

/// Optional numeric field: empty or malformed → `None`; non-finite values rejected.
pub fn num(s: &str) -> Option<f64> {
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

pub fn int(s: &str) -> Option<u32> {
    s.parse().ok()
}

/// Parse time from NMEA format (`HHMMSS` with optional `.s…` fraction).
pub fn parse_time(time_str: &str) -> Result<NaiveTime> {
    if !time_str.is_ascii() || time_str.len() < 6 {
        bail!("Invalid time string");
    }

    let hour: u32 = time_str[0..2].parse().context("Invalid hour")?;
    let minute: u32 = time_str[2..4].parse().context("Invalid minute")?;
    let second: u32 = time_str[4..6].parse().context("Invalid second")?;

    let fraction = &time_str[6..];
    let nanos = if fraction.is_empty() {
        0
    } else {
        let digits = fraction
            .strip_prefix('.')
            .context("Invalid fractional time")?;
        if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|c| c.is_ascii_digit()) {
            bail!("Invalid fractional time");
        }
        digits.parse::<u32>()? * 10_u32.pow(9 - digits.len() as u32)
    };
    NaiveTime::from_hms_nano_opt(hour, minute, second, nanos).context("Invalid time values")
}

/// Parse date from NMEA format (`DDMMYY`, years 2000–2099).
pub fn parse_date(date_str: &str) -> Result<NaiveDate> {
    if date_str.len() != 6 || !date_str.bytes().all(|c| c.is_ascii_digit()) {
        bail!("Invalid date string");
    }

    let day: u32 = date_str[0..2].parse()?;
    let month: u32 = date_str[2..4].parse()?;
    let year: i32 = date_str[4..6].parse::<i32>()? + 2000;

    NaiveDate::from_ymd_opt(year, month, day).context("Invalid date values")
}

/// Parse coordinate from NMEA format (`DDMM.MMMM` / `DDDMM.MMMM`) into signed degrees.
pub fn parse_coordinate(coord_str: &str, dir: &str) -> Result<f64> {
    let degree_digits = match dir {
        "N" | "S" => 2,
        "E" | "W" => 3,
        _ => bail!("Invalid coordinate direction"),
    };
    if !coord_str.is_ascii()
        || coord_str.find('.') != Some(degree_digits + 2)
        || !coord_str.bytes().all(|c| c.is_ascii_digit() || c == b'.')
    {
        bail!("Invalid coordinate format");
    }
    let degrees: f64 = coord_str[..degree_digits]
        .parse()
        .context("Invalid degrees")?;
    let minutes: f64 = coord_str[degree_digits..]
        .parse()
        .context("Invalid minutes")?;
    let maximum = if degree_digits == 2 { 90.0 } else { 180.0 };
    if !(0.0..60.0).contains(&minutes) || degrees > maximum || (degrees == maximum && minutes > 0.0)
    {
        bail!("Invalid coordinate range");
    }

    let decimal = degrees + minutes / 60.0;
    Ok(if dir == "S" || dir == "W" {
        -decimal
    } else {
        decimal
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn coordinates() {
        assert!((parse_coordinate("4807.038", "N").unwrap() - 48.1173).abs() < 0.001);
        assert!((parse_coordinate("01131.000", "W").unwrap() + 11.5166).abs() < 0.001);
        assert!(parse_coordinate("3351.000", "S").unwrap() < 0.0);
        for (c, d) in [
            ("", "N"),
            ("4807.038", "X"),
            ("9100.000", "N"),
            ("4860.000", "N"),
        ] {
            assert!(parse_coordinate(c, d).is_err(), "{c} {d}");
        }
    }

    #[test]
    fn times_and_dates() {
        let t = parse_time("123519").unwrap();
        assert_eq!((t.hour(), t.minute(), t.second()), (12, 35, 19));
        assert_eq!(parse_time("092750.25").unwrap().nanosecond(), 250_000_000);
        assert!(parse_time("123").is_err());
        assert!(parse_time("123519.").is_err());
        let d = parse_date("230323").unwrap();
        assert_eq!((d.year(), d.month(), d.day()), (2023, 3, 23));
        assert!(parse_date("230").is_err());
        assert!(parse_date("320123").is_err());
    }

    #[test]
    fn numbers() {
        assert_eq!(num("1.5"), Some(1.5));
        assert_eq!(num(""), None);
        assert_eq!(num("NaN"), None);
        assert_eq!(num("inf"), None);
    }
}
