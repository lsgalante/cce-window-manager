// Cell-aligned viewport panning.
//
// The desktop grid repeats every `period = cell_size + gap_width` virtual
// units, with cell k's rect starting at k*period. A pan offset is "aligned"
// when it is a multiple of the period: the viewport origin then sits exactly
// on a cell boundary, so the visible grid is in phase with the screen edge.
//
// `aligned_step` is the policy behind the PanLeft/Right/Up/Down actions: a
// press moves the viewport to the ADJACENT aligned offset in that direction —
// one full period from an aligned start, or just the remaining fraction from
// an unaligned one (a free-form pan re-aligns on the first keyed step rather
// than staying forever out of phase). The mechanism animates toward the
// returned value; repeated presses mid-flight should pass the current
// animation target as `current` so each press queues one more cell.

/// Offsets within this distance of an aligned value count as aligned — the
/// same tolerance the pan animation uses to snap onto its target, so a
/// finished animation's landing point steps a full period, never a crumb.
const ALIGN_EPSILON: f64 = 0.5;

/// The next cell-aligned pan offset from `current`, one step in `dir`
/// (negative = left/up, positive = right/down). Returns `current` unchanged
/// for a degenerate period or a zero direction.
pub fn aligned_step(current: f64, period: f64, dir: f64) -> f64 {
    if period <= 0.0 || dir == 0.0 {
        return current;
    }
    if dir > 0.0 {
        (((current + ALIGN_EPSILON) / period).floor() + 1.0) * period
    } else {
        (((current - ALIGN_EPSILON) / period).ceil() - 1.0) * period
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: f64 = 512.0;

    #[test]
    fn aligned_start_steps_a_full_period() {
        assert_eq!(aligned_step(0.0, P, 1.0), 512.0);
        assert_eq!(aligned_step(0.0, P, -1.0), -512.0);
        assert_eq!(aligned_step(-1024.0, P, 1.0), -512.0);
    }

    #[test]
    fn unaligned_start_realigns_first() {
        // Mid-cell pans land on the adjacent boundary, not a full period out.
        assert_eq!(aligned_step(100.0, P, 1.0), 512.0);
        assert_eq!(aligned_step(100.0, P, -1.0), 0.0);
        assert_eq!(aligned_step(-100.0, P, -1.0), -512.0);
    }

    #[test]
    fn near_aligned_counts_as_aligned() {
        // The animation stops within 0.5 of its target; a step from there
        // must cross a whole period, not crawl onto the boundary it's on.
        assert_eq!(aligned_step(511.9, P, 1.0), 1024.0);
        assert_eq!(aligned_step(512.1, P, -1.0), 0.0);
    }

    #[test]
    fn degenerate_inputs_are_inert() {
        assert_eq!(aligned_step(100.0, 0.0, 1.0), 100.0);
        assert_eq!(aligned_step(100.0, -5.0, 1.0), 100.0);
        assert_eq!(aligned_step(100.0, P, 0.0), 100.0);
    }
}
