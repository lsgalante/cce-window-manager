// Speed-ramp evaluation for duration-based camera transitions.
//
// The input is the DE-wide ramp spec string written by cce-ui's Ramp widget
// (`format_ramp_spec`): `"linear;0.000:0.100,0.500:1.000,1.000:0.050"` —
// keys are `time:speed` pairs in [0,1]², and the `smooth` head blends
// segments with smoothstep instead of linearly. The tiny parser and the
// per-segment interpolation are MIRRORED from cce-ui (this crate stays
// dependency-minimal), so the curve sculpted in the widget is exactly the
// curve evaluated here.
//
// The ramp is a SPEED profile over normalized time. Construction integrates
// it once into a cumulative-progress table normalized to end at exactly 1,
// so any profile arrives precisely at the target; zero-speed segments read
// as dwell. An (effectively) all-zero ramp yields `None` — callers fall
// back to their non-ramp animation.

/// Number of integration samples. Progress lookups interpolate linearly
/// between samples, so this bounds the timing error of a 60Hz animation to
/// well under a frame.
const SAMPLES: usize = 256;

/// Parse a ramp spec string into `(keys, smooth)`; `None` for anything that
/// doesn't yield at least two keys. Mirrors cce-ui's `parse_ramp_spec`.
pub fn parse_spec(spec: &str) -> Option<(Vec<(f32, f32)>, bool)> {
    let (head, body) = spec.split_once(';')?;
    let smooth = head.trim() == "smooth";
    let mut keys = Vec::new();
    for part in body.split(',') {
        let (p, v) = part.split_once(':')?;
        keys.push((
            p.trim().parse::<f32>().ok()?.clamp(0.0, 1.0),
            v.trim().parse::<f32>().ok()?.clamp(0.0, 1.0),
        ));
    }
    if keys.len() < 2 {
        return None;
    }
    keys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    Some((keys, smooth))
}

/// The ramp's value at `t` — endpoint-clamped, per-segment linear or
/// smoothstep blend. Mirrors cce-ui's `Ramp::get_interpolated_value`.
fn value_at(keys: &[(f32, f32)], smooth: bool, t: f32) -> f32 {
    if keys.is_empty() {
        return 0.0;
    }
    if t <= keys[0].0 {
        return keys[0].1;
    }
    if t >= keys[keys.len() - 1].0 {
        return keys[keys.len() - 1].1;
    }
    for i in 0..keys.len() - 1 {
        let (p1, v1) = keys[i];
        let (p2, v2) = keys[i + 1];
        if t >= p1 && t <= p2 {
            let range = p2 - p1;
            if range.abs() < 0.0001 {
                return v1;
            }
            let w = (t - p1) / range;
            let w = if smooth { w * w * (3.0 - 2.0 * w) } else { w };
            return v1 * (1.0 - w) + v2 * w;
        }
    }
    keys[0].1
}

/// A speed profile integrated into a normalized progress curve.
#[derive(Debug, Clone)]
pub struct SpeedRamp {
    /// Cumulative progress at SAMPLES+1 evenly spaced times:
    /// `table[0] == 0.0`, `table[SAMPLES] == 1.0`.
    table: Vec<f64>,
}

impl SpeedRamp {
    /// Build from a spec string; `None` if the spec doesn't parse or the
    /// speed integrates to (effectively) zero.
    pub fn from_spec(spec: &str) -> Option<SpeedRamp> {
        let (keys, smooth) = parse_spec(spec)?;
        // Midpoint rule per sample interval.
        let mut table = Vec::with_capacity(SAMPLES + 1);
        table.push(0.0);
        let mut acc = 0.0f64;
        for i in 0..SAMPLES {
            let mid = (i as f32 + 0.5) / SAMPLES as f32;
            acc += value_at(&keys, smooth, mid).max(0.0) as f64;
            table.push(acc);
        }
        let total = table[SAMPLES];
        if total < 1e-6 {
            return None;
        }
        for v in table.iter_mut() {
            *v /= total;
        }
        Some(SpeedRamp { table })
    }

    /// Progress through the transition at normalized time `t` (clamped to
    /// [0,1]): 0 at start, exactly 1 at the end, monotonic.
    pub fn progress(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let x = t * SAMPLES as f64;
        let i = x.floor() as usize;
        let frac = x - i as f64;
        self.table[i] * (1.0 - frac) + self.table[i + 1] * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_speed_is_linear_progress() {
        let r = SpeedRamp::from_spec("linear;0.0:1.0,1.0:1.0").unwrap();
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert!((r.progress(t) - t).abs() < 1e-3, "t={t}");
        }
    }

    #[test]
    fn endpoints_are_exact() {
        let r = SpeedRamp::from_spec("smooth;0.0:0.1,0.4:1.0,1.0:0.05").unwrap();
        assert_eq!(r.progress(0.0), 0.0);
        assert_eq!(r.progress(1.0), 1.0);
        assert_eq!(r.progress(-0.5), 0.0);
        assert_eq!(r.progress(2.0), 1.0);
    }

    #[test]
    fn slow_start_covers_less_ground_early() {
        // Speed ramps 0 → 1: the first half of the time covers well under
        // half the distance.
        let r = SpeedRamp::from_spec("linear;0.0:0.0,1.0:1.0").unwrap();
        assert!(r.progress(0.5) < 0.3, "got {}", r.progress(0.5));
    }

    #[test]
    fn monotonic_even_with_dwell() {
        // A zero-speed plateau mid-ramp: progress holds but never regresses.
        let r = SpeedRamp::from_spec("linear;0.0:1.0,0.4:0.0,0.6:0.0,1.0:1.0").unwrap();
        let mut last = 0.0;
        for i in 0..=100 {
            let p = r.progress(i as f64 / 100.0);
            assert!(p >= last - 1e-12);
            last = p;
        }
        // The plateau really dwells: progress barely moves across it.
        assert!((r.progress(0.58) - r.progress(0.42)).abs() < 0.02);
    }

    #[test]
    fn zero_ramp_is_rejected() {
        assert!(SpeedRamp::from_spec("linear;0.0:0.0,1.0:0.0").is_none());
        assert!(SpeedRamp::from_spec("garbage").is_none());
        assert!(SpeedRamp::from_spec("linear;0.5:1.0").is_none());
    }

    #[test]
    fn smooth_matches_widget_semantics() {
        // One segment 0→1 with smoothstep: value at midpoint is 0.5 (the
        // blend is symmetric), and the curve is steeper mid-segment than
        // linear at the edges.
        let (keys, smooth) = parse_spec("smooth;0.0:0.0,1.0:1.0").unwrap();
        assert!(smooth);
        assert!((value_at(&keys, true, 0.5) - 0.5).abs() < 1e-6);
        assert!(value_at(&keys, true, 0.25) < 0.25);
        assert!(value_at(&keys, true, 0.75) > 0.75);
    }
}
