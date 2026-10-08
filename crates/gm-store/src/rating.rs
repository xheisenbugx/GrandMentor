//! Glicko-2-lite puzzle rating.
//!
//! A single-game Glicko update (the Glicko-2 rating step without volatility iteration) where the
//! puzzle is an opponent with a fixed, fairly certain rating. The player's rating deviation (RD)
//! starts at 350 and shrinks with every attempt down to a floor, so early results move the rating
//! a lot (fast calibration) and later results move it gently, like chess.com puzzle ratings.

use std::f64::consts::PI;

pub const START_RATING: f64 = 1200.0;
pub const START_RD: f64 = 350.0;
/// RD never drops below this, so the rating keeps responding.
pub const MIN_RD: f64 = 60.0;
/// Puzzles are treated as opponents with this deviation.
pub const PUZZLE_RD: f64 = 80.0;
pub const MIN_RATING: f64 = 100.0;
pub const MAX_RATING: f64 = 3500.0;
/// Largest change from a single attempt.
pub const MAX_DELTA: f64 = 100.0;

const Q: f64 = std::f64::consts::LN_10 / 400.0;

fn g(rd: f64) -> f64 {
    1.0 / (1.0 + 3.0 * Q * Q * rd * rd / (PI * PI)).sqrt()
}

/// Expected score of a player rated `r` against a puzzle rated `rp`.
pub fn expected(r: f64, rp: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-g(PUZZLE_RD) * (r - rp) / 400.0))
}

/// Returns `(new_rating, new_rd)`.
///
/// Guarantees: solving never lowers the rating and failing never raises it (monotonic), each
/// rated attempt moves the rating by at least one point in the right direction (clear feedback
/// for beginners) unless clamped at the bounds, and the result is always finite and in range.
pub fn update(rating: f64, rd: f64, puzzle_rating: f64, solved: bool) -> (f64, f64) {
    let r = if rating.is_finite() {
        rating.clamp(MIN_RATING, MAX_RATING)
    } else {
        START_RATING
    };
    let rd = if rd.is_finite() {
        rd.clamp(MIN_RD, START_RD)
    } else {
        START_RD
    };
    let rp = if puzzle_rating.is_finite() {
        puzzle_rating.clamp(MIN_RATING, MAX_RATING)
    } else {
        START_RATING
    };

    let gp = g(PUZZLE_RD);
    let e = expected(r, rp).clamp(1e-6, 1.0 - 1e-6);
    let d2 = 1.0 / (Q * Q * gp * gp * e * (1.0 - e));
    let s = if solved { 1.0 } else { 0.0 };
    let denom = 1.0 / (rd * rd) + 1.0 / d2;
    let mut delta = (Q / denom * gp * (s - e)).clamp(-MAX_DELTA, MAX_DELTA);
    if solved && delta < 1.0 {
        delta = 1.0;
    } else if !solved && delta > -1.0 {
        delta = -1.0;
    }
    let new_r = (r + delta).clamp(MIN_RATING, MAX_RATING);
    let new_rd = (1.0 / denom).sqrt().clamp(MIN_RD, START_RD);
    (new_r, new_rd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_and_bounded() {
        for &r in &[100.0, 800.0, 1200.0, 2000.0, 3500.0] {
            for &rd in &[350.0, 200.0, 60.0] {
                for &p in &[400.0, 1200.0, 2800.0] {
                    let (win, rd1) = update(r, rd, p, true);
                    let (loss, rd2) = update(r, rd, p, false);
                    assert!(win >= r && loss <= r, "r={r} rd={rd} p={p}");
                    assert!(rd1 <= rd && rd2 <= rd);
                    assert!((MIN_RATING..=MAX_RATING).contains(&win));
                    assert!((MIN_RATING..=MAX_RATING).contains(&loss));
                }
            }
        }
        // Harder puzzles reward more.
        let (easy, _) = update(1200.0, 200.0, 900.0, true);
        let (hard, _) = update(1200.0, 200.0, 1600.0, true);
        assert!(hard > easy);
        // New players move faster than calibrated ones.
        let (fresh, _) = update(1200.0, 350.0, 1200.0, true);
        let (settled, _) = update(1200.0, 60.0, 1200.0, true);
        assert!(fresh - 1200.0 > settled - 1200.0);
        assert!(update(f64::NAN, f64::NAN, f64::NAN, true).0.is_finite());
    }
}
