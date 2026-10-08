//! Elo -> search & selection parameters (piecewise-linear interpolation of a calibration table).

/// Human-like strength parameters for a given Elo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strength {
    /// Maximum search depth (plies).
    pub depth: u8,
    /// Node budget for the search.
    pub nodes: u64,
    /// Hard wall-clock cap for the search (ms).
    pub movetime_ms: u64,
    /// Number of candidate lines (MultiPV) the bot considers.
    pub multipv: usize,
    /// Softmax temperature in centipawns (higher = more random among candidates).
    pub temperature_cp: f64,
    /// Probability of an "oversight": choosing by a naive one-ply look that ignores replies
    /// (hangs pieces, misses opponent threats).
    pub oversight: f64,
    /// Probability of not noticing captures available this move.
    pub miss_capture: f64,
    /// Never play a candidate more than this many centipawns worse than the best line.
    pub max_loss_cp: i32,
    /// Plies the bot will follow the opening book for.
    pub book_plies: usize,
    /// Probability of leaving the book early on any given move.
    pub book_exit: f64,
    /// Candidate scores are clamped to +-this (so weak bots don't always see mates as "infinite").
    pub score_cap: i32,
    /// Multiplier for style preference bonuses.
    pub style_weight: f64,
}

// elo, depth, nodes, movetime, multipv, temp, oversight, miss_capture, max_loss, book_plies, book_exit, style
type Row = (f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64);
const TABLE: &[Row] = &[
    (250.0, 1.0, 1_500.0, 60.0, 8.0, 320.0, 0.42, 0.40, 10_000.0, 4.0, 0.35, 1.6),
    (600.0, 2.0, 6_000.0, 120.0, 8.0, 190.0, 0.26, 0.22, 1_500.0, 6.0, 0.25, 1.4),
    (1000.0, 3.0, 25_000.0, 200.0, 6.0, 110.0, 0.13, 0.10, 800.0, 10.0, 0.15, 1.2),
    (1400.0, 4.0, 70_000.0, 300.0, 5.0, 62.0, 0.06, 0.04, 450.0, 14.0, 0.10, 1.0),
    (1800.0, 6.0, 200_000.0, 450.0, 4.0, 34.0, 0.025, 0.01, 220.0, 18.0, 0.06, 0.9),
    (2200.0, 8.0, 500_000.0, 700.0, 4.0, 17.0, 0.006, 0.0, 110.0, 22.0, 0.04, 0.7),
    (2600.0, 11.0, 1_500_000.0, 1_000.0, 3.0, 7.0, 0.0, 0.0, 50.0, 26.0, 0.02, 0.5),
    (3000.0, 24.0, 4_000_000.0, 1_500.0, 2.0, 1.0, 0.0, 0.0, 15.0, 30.0, 0.0, 0.3),
];

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

impl Strength {
    pub fn for_elo(elo: u16) -> Strength {
        let e = f64::from(elo).clamp(TABLE[0].0, TABLE[TABLE.len() - 1].0);
        let mut lo = TABLE[0];
        let mut hi = TABLE[TABLE.len() - 1];
        for w in TABLE.windows(2) {
            if e >= w[0].0 && e <= w[1].0 {
                lo = w[0];
                hi = w[1];
                break;
            }
        }
        let span = hi.0 - lo.0;
        let t = if span > 0.0 { (e - lo.0) / span } else { 0.0 };
        let depth = lerp(lo.1, hi.1, t).round().clamp(1.0, 64.0) as u8;
        // Geometric interpolation for node counts.
        let nodes = (lo.2.ln() + (hi.2.ln() - lo.2.ln()) * t).exp().round() as u64;
        Strength {
            depth,
            nodes: nodes.max(500),
            movetime_ms: lerp(lo.3, hi.3, t).round().max(20.0) as u64,
            multipv: lerp(lo.4, hi.4, t).round().clamp(1.0, 10.0) as usize,
            temperature_cp: lerp(lo.5, hi.5, t).max(0.5),
            oversight: lerp(lo.6, hi.6, t).clamp(0.0, 1.0),
            miss_capture: lerp(lo.7, hi.7, t).clamp(0.0, 1.0),
            max_loss_cp: lerp(lo.8, hi.8, t).round() as i32,
            book_plies: lerp(lo.9, hi.9, t).round().max(0.0) as usize,
            book_exit: lerp(lo.10, hi.10, t).clamp(0.0, 1.0),
            score_cap: 400 + i32::from(elo) / 2 + if elo >= 2000 { 20_000 } else { 0 },
            style_weight: lerp(lo.11, hi.11, t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_strength() {
        let mut prev = Strength::for_elo(100);
        for elo in (300..=3200).step_by(50) {
            let s = Strength::for_elo(elo);
            assert!(s.depth >= prev.depth, "depth at {elo}");
            assert!(s.nodes >= prev.nodes, "nodes at {elo}");
            assert!(s.temperature_cp <= prev.temperature_cp, "temp at {elo}");
            assert!(s.oversight <= prev.oversight, "oversight at {elo}");
            assert!(s.max_loss_cp <= prev.max_loss_cp, "max_loss at {elo}");
            prev = s;
        }
    }
}
