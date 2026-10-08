//! Lichess-style accuracy and an accuracy -> Elo heuristic.

/// Accuracy (0..100) of a single move given the mover's win% before and after it.
/// Lichess: `103.1668 * exp(-0.04354 * diff) - 3.1669`, plus a +1 uncertainty bonus.
pub fn move_accuracy(win_before: f32, win_after: f32) -> f32 {
    if !win_before.is_finite() || !win_after.is_finite() {
        return 0.0;
    }
    if win_after >= win_before {
        return 100.0;
    }
    let diff = (win_before - win_after) as f64;
    let raw = 103.166_810_071_164_9 * (-0.043_544_153_867_539_51 * diff).exp() - 3.166_924_740_191_411;
    ((raw + 1.0).clamp(0.0, 100.0)) as f32
}

fn std_dev(xs: &[f32]) -> f32 {
    if xs.is_empty() {
        return 0.0;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().map(|&x| x as f64).sum::<f64>() / n;
    let var = xs.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / n;
    var.sqrt() as f32
}

/// Per-side accuracy, lichess method.
///
/// `white_wins[i]` is white's win% (0..100) in position `i` (len = moves + 1).
/// `move_acc[i]` is the accuracy of move `i` (already computed for the mover).
/// `white_first` is true when white made move 0.
/// Returns `(white_accuracy, black_accuracy)`; a side with no moves gets 100.
pub fn game_accuracy(white_wins: &[f32], move_acc: &[f32], white_first: bool) -> (f32, f32) {
    let n_moves = move_acc.len().min(white_wins.len().saturating_sub(1));
    if n_moves == 0 {
        return (100.0, 100.0);
    }
    let wins = &white_wins[..=n_moves];
    let window = (n_moves / 10).clamp(2, 8).min(wins.len());
    // Volatility weights: (window - 2) copies of the first window, then sliding windows.
    let mut weights: Vec<f32> = Vec::with_capacity(n_moves);
    let first = std_dev(&wins[..window]).clamp(0.5, 12.0);
    for _ in 0..window.saturating_sub(2) {
        weights.push(first);
    }
    for w in wins.windows(window) {
        weights.push(std_dev(w).clamp(0.5, 12.0));
    }

    let mut acc = [Vec::with_capacity(n_moves / 2 + 1), Vec::with_capacity(n_moves / 2 + 1)];
    for (i, &a) in move_acc.iter().take(n_moves).enumerate() {
        let white_moved = (i % 2 == 0) == white_first;
        let weight = weights.get(i).copied().unwrap_or(0.5);
        acc[usize::from(!white_moved)].push((a, weight));
    }
    let side = |xs: &[(f32, f32)]| -> f32 {
        if xs.is_empty() {
            return 100.0;
        }
        let wsum: f64 = xs.iter().map(|&(_, w)| w as f64).sum();
        let weighted = if wsum > 0.0 {
            xs.iter().map(|&(a, w)| a as f64 * w as f64).sum::<f64>() / wsum
        } else {
            xs.iter().map(|&(a, _)| a as f64).sum::<f64>() / xs.len() as f64
        };
        // Harmonic mean; floor tiny values so a single 0 doesn't collapse everything.
        let inv: f64 = xs.iter().map(|&(a, _)| 1.0 / (a as f64).max(0.5)).sum();
        let harmonic = xs.len() as f64 / inv;
        (((weighted + harmonic) / 2.0).clamp(0.0, 100.0)) as f32
    };
    (side(&acc[0]), side(&acc[1]))
}

/// Rough "game rating" estimate from accuracy (chess.com-like). Short games are pulled toward
/// a neutral 1200 because a handful of moves says little about strength.
pub fn estimate_elo(accuracy: f32, moves_by_side: usize) -> u16 {
    const TABLE: [(f32, f32); 12] = [
        (0.0, 100.0),
        (20.0, 200.0),
        (40.0, 400.0),
        (50.0, 600.0),
        (60.0, 850.0),
        (70.0, 1150.0),
        (77.0, 1400.0),
        (83.0, 1700.0),
        (88.0, 2000.0),
        (92.0, 2300.0),
        (96.0, 2650.0),
        (100.0, 3000.0),
    ];
    let a = if accuracy.is_finite() { accuracy.clamp(0.0, 100.0) } else { 0.0 };
    let mut raw = TABLE[TABLE.len() - 1].1;
    for pair in TABLE.windows(2) {
        let (x0, y0) = pair[0];
        let (x1, y1) = pair[1];
        if a <= x1 {
            raw = y0 + (y1 - y0) * (a - x0) / (x1 - x0);
            break;
        }
    }
    let confidence = (moves_by_side as f32 / 15.0).clamp(0.0, 1.0);
    let elo = 1200.0 + (raw - 1200.0) * confidence;
    elo.round().clamp(100.0, 3200.0) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_accuracy_bounds() {
        assert_eq!(move_accuracy(50.0, 60.0), 100.0);
        assert!(move_accuracy(50.0, 50.0) >= 99.9);
        let small = move_accuracy(60.0, 55.0);
        let big = move_accuracy(60.0, 10.0);
        assert!(small > 75.0 && small < 100.0, "{small}");
        assert!(big < 15.0, "{big}");
        assert!(move_accuracy(100.0, 0.0) >= 0.0);
    }

    #[test]
    fn perfect_game_is_100() {
        let wins = vec![50.0; 21];
        let acc = vec![100.0; 20];
        let (w, b) = game_accuracy(&wins, &acc, true);
        assert!((w - 100.0).abs() < 0.01 && (b - 100.0).abs() < 0.01);
    }

    #[test]
    fn one_blunder_hurts() {
        let mut acc = vec![100.0; 20];
        acc[5] = 5.0; // black blunder
        let mut wins = vec![50.0; 21];
        for w in wins.iter_mut().skip(6) {
            *w = 95.0;
        }
        let (w, b) = game_accuracy(&wins, &acc, true);
        assert!(w > 99.0);
        assert!(b < 80.0, "{b}");
    }

    #[test]
    fn elo_monotonic() {
        let mut last = 0;
        for a in (0..=100).step_by(5) {
            let e = estimate_elo(a as f32, 40);
            assert!(e >= last);
            last = e;
        }
        assert_eq!(estimate_elo(f32::NAN, 40), 100);
        // Short games regress toward 1200.
        assert!(estimate_elo(100.0, 2) < estimate_elo(100.0, 40));
    }
}
