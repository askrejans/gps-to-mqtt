//! Static hold: pin the reported position while the vehicle is stationary.
//!
//! GNSS positions wander by a few metres even when nothing moves. With hold
//! enabled, fixes slower than the speed threshold report the running mean of
//! the stationary samples instead of the raw position. The hold releases as
//! soon as speed rises or the raw position leaves the radius, so real movement
//! is never masked by more than `radius_m`.

use crate::models::Fix;
use crate::telemetry::haversine;

/// Samples beyond this weight are blended as an exponential moving average.
const MAX_SAMPLES: f64 = 120.0;
/// The reported position only moves when the internal mean has moved this far,
/// so a parked vehicle reports one constant position (no last-digit churn).
const REPORT_STEP_M: f64 = 1.0;

#[derive(Debug, Clone, Copy)]
pub struct HoldConfig {
    pub speed_mps: f64,
    pub radius_m: f64,
}

#[derive(Debug)]
struct Anchor {
    lat: f64,
    lon: f64,
    alt: Option<f64>,
    weight: f64,
}

#[derive(Debug)]
pub struct StaticHold {
    config: HoldConfig,
    anchor: Option<Anchor>,
    /// Last reported (lat, lon, alt) while holding.
    shown: Option<(f64, f64, Option<f64>)>,
}

impl StaticHold {
    pub fn new(config: HoldConfig) -> Self {
        Self {
            config,
            anchor: None,
            shown: None,
        }
    }

    /// Apply hold to a fix in place; sets `position_held`.
    pub fn apply(&mut self, fix: &mut Fix) {
        let Some((lat, lon)) = fix.position() else {
            self.anchor = None;
            self.shown = None;
            return;
        };
        let slow = fix.speed_mps.is_none_or(|v| v < self.config.speed_mps);
        if !slow {
            self.anchor = None;
            self.shown = None;
            return;
        }
        let anchor = match self.anchor.take() {
            Some(a) if haversine(a.lat, a.lon, lat, lon) <= self.config.radius_m => {
                let w = (a.weight + 1.0).min(MAX_SAMPLES);
                let k = 1.0 / w;
                Anchor {
                    lat: a.lat + (lat - a.lat) * k,
                    lon: a.lon + (lon - a.lon) * k,
                    alt: match (a.alt, fix.altitude_m) {
                        (Some(old), Some(new)) => Some(old + (new - old) * k),
                        (old, new) => new.or(old),
                    },
                    weight: w,
                }
            }
            // First stationary fix, or drifted outside the radius: re-anchor.
            _ => Anchor {
                lat,
                lon,
                alt: fix.altitude_m,
                weight: 1.0,
            },
        };
        let reanchored = anchor.weight == 1.0;
        let shown = match self.shown {
            Some(s)
                if !reanchored && haversine(s.0, s.1, anchor.lat, anchor.lon) < REPORT_STEP_M =>
            {
                s
            }
            _ => (anchor.lat, anchor.lon, anchor.alt),
        };
        self.shown = Some(shown);
        fix.latitude = Some(shown.0);
        fix.longitude = Some(shown.1);
        if shown.2.is_some() {
            fix.altitude_m = shown.2;
        }
        fix.speed_mps = fix.speed_mps.map(|_| 0.0);
        fix.course_deg = None;
        fix.position_held = true;
        self.anchor = Some(anchor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FixQuality;

    fn fix(lat: f64, lon: f64, speed: f64) -> Fix {
        Fix {
            valid: true,
            quality: FixQuality::Gps,
            latitude: Some(lat),
            longitude: Some(lon),
            altitude_m: Some(10.0),
            speed_mps: Some(speed),
            course_deg: Some(123.0),
            ..Fix::default()
        }
    }

    fn hold() -> StaticHold {
        StaticHold::new(HoldConfig {
            speed_mps: 2.0 / 3.6,
            radius_m: 5.0,
        })
    }

    #[test]
    fn stationary_jitter_is_averaged_and_flagged() {
        let mut h = hold();
        // ±1 m jitter around a point (1e-5° latitude ≈ 1.1 m).
        let mut out = Vec::new();
        for (i, d) in [0.0, 1e-5, -1e-5, 0.8e-5, -0.8e-5].iter().enumerate() {
            let mut f = fix(56.9637 + d, 24.2289, 0.1 + i as f64 * 0.01);
            h.apply(&mut f);
            assert!(f.position_held);
            assert_eq!(f.speed_mps, Some(0.0));
            assert_eq!(f.course_deg, None);
            out.push(f.latitude.unwrap());
        }
        let spread = out.iter().cloned().fold(f64::MIN, f64::max)
            - out.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            spread < 0.6e-5,
            "held output moves less than raw ({spread})"
        );
        // Sub-metre refinements of the mean are not reported: output is constant.
        // (The first samples may step while a two-sample mean settles.)
        assert!(out[2..].windows(2).all(|w| w[0] == w[1]), "{out:?}");
    }

    #[test]
    fn releases_on_speed_and_on_drift() {
        let mut h = hold();
        let mut f = fix(56.9637, 24.2289, 0.1);
        h.apply(&mut f);
        // Moving: raw values pass through.
        let mut moving = fix(56.96371, 24.2289, 5.0);
        h.apply(&mut moving);
        assert!(!moving.position_held);
        assert_eq!(moving.latitude, Some(56.96371));
        assert_eq!(moving.course_deg, Some(123.0));
        // Slow but 11 m away from the anchor: re-anchor at the new point.
        let mut a = fix(56.9637, 24.2289, 0.1);
        h.apply(&mut a);
        let mut far = fix(56.9638, 24.2289, 0.1);
        h.apply(&mut far);
        assert_eq!(far.latitude, Some(56.9638));
        assert!(far.position_held);
    }

    #[test]
    fn invalid_fix_resets_and_is_untouched() {
        let mut h = hold();
        let mut f = fix(56.9637, 24.2289, 0.1);
        f.valid = false;
        h.apply(&mut f);
        assert!(!f.position_held);
        assert_eq!(f.course_deg, Some(123.0));
    }
}
