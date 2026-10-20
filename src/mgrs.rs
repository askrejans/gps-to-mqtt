//! WGS-84 → UTM → MGRS (Military Grid Reference System).
//!
//! Precision follows the military convention: 10 digits = 1 m, 8 digits = 10 m.
//! Digits are truncated, not rounded, so a reference always names the square
//! that contains the point. MGRS covers 80°S – 84°N; the polar UPS areas return
//! `None`.

const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_223_563;
const K0: f64 = 0.9996;
const BANDS: &[u8] = b"CDEFGHJKLMNPQRSTUVWX";
const COLUMN_SETS: [&[u8]; 3] = [b"ABCDEFGH", b"JKLMNPQR", b"STUVWXYZ"];
const ROWS: &[u8] = b"ABCDEFGHJKLMNPQRSTUV";

/// UTM coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Utm {
    pub zone: u8,
    pub band: char,
    pub easting: f64,
    pub northing: f64,
}

fn zone_number(lat: f64, lon: f64) -> u8 {
    // Norway: zone 32 widened to 3°E – 12°E between 56°N and 64°N.
    if (56.0..64.0).contains(&lat) && (3.0..12.0).contains(&lon) {
        return 32;
    }
    // Svalbard: zones 31, 33, 35, 37 only, between 72°N and 84°N.
    if (72.0..=84.0).contains(&lat) && (0.0..42.0).contains(&lon) {
        return match lon {
            l if l < 9.0 => 31,
            l if l < 21.0 => 33,
            l if l < 33.0 => 35,
            _ => 37,
        };
    }
    (((lon + 180.0) / 6.0).floor() as i32).clamp(0, 59) as u8 + 1
}

fn band(lat: f64) -> char {
    // 8° bands from 80°S; X is stretched to 12° (72°N – 84°N).
    let i = (((lat + 80.0) / 8.0).floor() as usize).min(BANDS.len() - 1);
    BANDS[i] as char
}

/// Transverse Mercator projection (Snyder, USGS PP 1395), WGS-84.
pub fn to_utm(lat: f64, lon: f64) -> Option<Utm> {
    if !(lat.is_finite() && lon.is_finite())
        || !(-80.0..=84.0).contains(&lat)
        || !(-180.0..=180.0).contains(&lon)
    {
        return None;
    }
    let lon = if lon >= 180.0 { lon - 360.0 } else { lon };
    let zone = zone_number(lat, lon);
    let lon0 = (f64::from(zone) - 1.0) * 6.0 - 180.0 + 3.0;

    let e2 = F * (2.0 - F);
    let ep2 = e2 / (1.0 - e2);
    let (e4, e6) = (e2 * e2, e2 * e2 * e2);
    let phi = lat.to_radians();
    let (sin, cos, tan) = (phi.sin(), phi.cos(), phi.tan());

    let n = A / (1.0 - e2 * sin * sin).sqrt();
    let t = tan * tan;
    let c = ep2 * cos * cos;
    let a = cos * (lon - lon0).to_radians();
    let m = A
        * ((1.0 - e2 / 4.0 - 3.0 * e4 / 64.0 - 5.0 * e6 / 256.0) * phi
            - (3.0 * e2 / 8.0 + 3.0 * e4 / 32.0 + 45.0 * e6 / 1024.0) * (2.0 * phi).sin()
            + (15.0 * e4 / 256.0 + 45.0 * e6 / 1024.0) * (4.0 * phi).sin()
            - (35.0 * e6 / 3072.0) * (6.0 * phi).sin());

    let easting = K0
        * n
        * (a + (1.0 - t + c) * a.powi(3) / 6.0
            + (5.0 - 18.0 * t + t * t + 72.0 * c - 58.0 * ep2) * a.powi(5) / 120.0)
        + 500_000.0;
    let mut northing = K0
        * (m + n
            * tan
            * (a * a / 2.0
                + (5.0 - t + 9.0 * c + 4.0 * c * c) * a.powi(4) / 24.0
                + (61.0 - 58.0 * t + t * t + 600.0 * c - 330.0 * ep2) * a.powi(6) / 720.0));
    if lat < 0.0 {
        northing += 10_000_000.0;
    }
    Some(Utm {
        zone,
        band: band(lat),
        easting,
        northing,
    })
}

/// MGRS reference with `digits` total numerical digits (2–10, even), e.g.
/// `35V LC 14005 58201` (10) or `35V LC 1400 5820` (8).
pub fn mgrs(lat: f64, lon: f64, digits: u8) -> Option<String> {
    let utm = to_utm(lat, lon)?;
    let per_axis = (digits.clamp(2, 10) / 2) as u32;
    let set = (usize::from(utm.zone) - 1) % 3;

    let col_index = (utm.easting / 100_000.0).floor() as i64;
    let column = *COLUMN_SETS[set].get(usize::try_from(col_index - 1).ok()?)? as char;
    let row_offset = if utm.zone % 2 == 0 { 5 } else { 0 };
    let row_index =
        ((utm.northing / 100_000.0).floor() as i64 + row_offset).rem_euclid(20) as usize;
    let row = ROWS[row_index] as char;

    let divisor = 10f64.powi(5 - per_axis as i32);
    let e = ((utm.easting.rem_euclid(100_000.0)) / divisor).floor() as u64;
    let n = ((utm.northing.rem_euclid(100_000.0)) / divisor).floor() as u64;
    let w = per_axis as usize;
    Some(format!(
        "{}{} {}{} {:0w$} {:0w$}",
        utm.zone, utm.band, column, row, e, n
    ))
}

/// Reduce a 10-digit reference to 8 digits (1 m → 10 m) by truncation.
pub fn to_8_digit(ten: &str) -> Option<String> {
    let mut parts = ten.split(' ');
    let (gzd, square, e, n) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    (e.len() == 5 && n.len() == 5).then(|| format!("{gzd} {square} {} {}", &e[..4], &n[..4]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_reference_point() {
        // The canonical check value: 0°N 0°E.
        assert_eq!(mgrs(0.0, 0.0, 10).as_deref(), Some("31N AA 66021 00000"));
        let u = to_utm(0.0, 0.0).unwrap();
        assert!((u.easting - 166_021.443).abs() < 0.01, "{}", u.easting);
    }

    #[test]
    fn central_meridian_matches_meridian_arc() {
        // On a central meridian easting is 500 km and northing is k0 × meridian arc.
        let u = to_utm(45.0, 3.0).unwrap();
        assert!((u.easting - 500_000.0).abs() < 1e-6);
        assert!((u.northing - 4_982_950.4).abs() < 0.1, "{}", u.northing);
        let s = to_utm(-45.0, 3.0).unwrap();
        assert!((s.northing - (10_000_000.0 - 4_982_950.4)).abs() < 0.1);
    }

    #[test]
    fn washington_monument() {
        // 38.8895°N 77.0353°W, cross-checked against an independent Krüger
        // n-series implementation (E 323478.063, N 4306483.242).
        assert_eq!(
            mgrs(38.8895, -77.0353, 10).as_deref(),
            Some("18S UJ 23478 06483")
        );
        assert_eq!(
            mgrs(38.8895, -77.0353, 8).as_deref(),
            Some("18S UJ 2347 0648")
        );
    }

    #[test]
    fn matches_independent_kruger_series() {
        // Reference values from a separate Krüger n-series (Karney 2011) script.
        for (lat, lon, zone, e, n) in [
            (38.8895, -77.0353, 18, 323_478.063, 4_306_483.242),
            (56.9637, 24.2289, 35, 331_524.319, 6_316_761.712),
            (-33.8568, 151.2153, 56, 334_900.570, 6_252_288.753),
            (60.0, 5.0, 32, 276_979.926, 6_658_157.202), // 4° west of the widened zone's meridian
            (78.0, 15.0, 33, 500_000.000, 8_658_369.586),
        ] {
            let u = to_utm(lat, lon).unwrap();
            assert_eq!(u.zone, zone);
            assert!(
                (u.easting - e).abs() < 0.05,
                "{lat},{lon}: E {} vs {e}",
                u.easting
            );
            assert!(
                (u.northing - n).abs() < 0.05,
                "{lat},{lon}: N {} vs {n}",
                u.northing
            );
        }
    }

    #[test]
    fn zone_exceptions_and_bands() {
        assert_eq!(to_utm(60.0, 5.0).unwrap().zone, 32); // Norway widening
        assert_eq!(to_utm(60.0, 2.0).unwrap().zone, 31);
        assert_eq!(to_utm(78.0, 8.0).unwrap().zone, 31); // Svalbard
        assert_eq!(to_utm(78.0, 15.0).unwrap().zone, 33);
        assert_eq!(to_utm(78.0, 25.0).unwrap().zone, 35);
        assert_eq!(to_utm(78.0, 40.0).unwrap().zone, 37);
        assert_eq!(to_utm(83.9, 10.0).unwrap().band, 'X');
        assert_eq!(to_utm(-79.9, 10.0).unwrap().band, 'C');
        assert_eq!(to_utm(56.9637, 24.2289).unwrap().zone, 35);
        assert!(mgrs(85.0, 0.0, 10).is_none(), "UPS area");
        assert!(mgrs(-81.0, 0.0, 10).is_none(), "UPS area");
        assert!(mgrs(f64::NAN, 0.0, 10).is_none());
    }

    #[test]
    fn precisions_truncate() {
        let ten = mgrs(56.9637, 24.2289, 10).unwrap();
        let eight = mgrs(56.9637, 24.2289, 8).unwrap();
        assert_eq!(to_8_digit(&ten).as_deref(), Some(eight.as_str()));
        assert!(ten.starts_with("35V "), "{ten}");
        assert_eq!(ten.len(), "35V LC 12345 67890".len());
        assert_eq!(eight.len(), "35V LC 1234 6789".len());
        assert!(to_8_digit("garbage").is_none());
    }

    #[test]
    fn hemispheres_and_antimeridian() {
        let south = mgrs(-33.8568, 151.2153, 10).unwrap(); // Sydney Opera House
        assert!(south.starts_with("56H "), "{south}");
        assert!(mgrs(10.0, 179.9999, 10).unwrap().starts_with("60P "));
        assert!(mgrs(10.0, -179.9999, 10).unwrap().starts_with("1P "));
        assert!(mgrs(10.0, 180.0, 10).unwrap().starts_with("1P "));
    }
}
