//! Rating fit for the bot calibration harness (`gm-calibrate`, see `docs/BOT_CALIBRATION.md`).
//!
//! Model: the Elo logistic (Bradley–Terry with base 10 and a 400-point scale),
//! `P(i beats j) = 1 / (1 + 10^((r_j - r_i) / 400))`, draws counted as half a win for each side.
//! Ratings are the maximum-likelihood solution over all games (Newton's method), with a weak
//! prior of half a virtual draw per pairing that was played (keeps a 100% score finite). The scale
//! has no natural zero, so the caller fixes it with an anchor (see [`anchor_to_mean`]).
//! Confidence intervals come from a bootstrap that resamples game *pairs* (each opening is played
//! twice with colours reversed, so the two games of a pair are not independent).

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// One finished game: `white`/`black` index into the player list, `score` is White's score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameResult {
    pub white: usize,
    pub black: usize,
    /// 1.0 White won, 0.5 draw, 0.0 Black won.
    pub score: f64,
    /// Games sharing a `pair` id were played from the same opening with colours reversed.
    pub pair: usize,
}

const K: f64 = std::f64::consts::LN_10 / 400.0;
/// Virtual draws added per pairing that was actually played.
const PRIOR_DRAWS: f64 = 0.5;
const MAX_ITERS: usize = 100;

/// Expected score of a player rated `a` against one rated `b`.
pub fn expected(a: f64, b: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((b - a) / 400.0))
}

/// Maximum-likelihood Elo ratings for `n` players (mean 0). Players without games get 0.
pub fn fit(n: usize, games: &[GameResult]) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    // Pairwise totals: s[i][j] = points i scored against j, g[i][j] = games between them.
    let mut s = vec![vec![0.0f64; n]; n];
    let mut g = vec![vec![0.0f64; n]; n];
    for r in games {
        if r.white >= n || r.black >= n || r.white == r.black {
            continue;
        }
        s[r.white][r.black] += r.score;
        s[r.black][r.white] += 1.0 - r.score;
        g[r.white][r.black] += 1.0;
        g[r.black][r.white] += 1.0;
    }
    for i in 0..n {
        for j in 0..n {
            if g[i][j] > 0.0 {
                g[i][j] += PRIOR_DRAWS;
                s[i][j] += PRIOR_DRAWS / 2.0;
            }
        }
    }
    let active: Vec<usize> = (0..n).filter(|&i| g[i].iter().any(|&x| x > 0.0)).collect();
    let mut r = vec![0.0f64; n];
    if active.len() < 2 {
        return r;
    }
    // Newton on the active players, with active[0] pinned to 0 to remove the free offset.
    let free: Vec<usize> = active[1..].to_vec();
    let m = free.len();
    for _ in 0..MAX_ITERS {
        let mut grad = vec![0.0f64; m];
        let mut hess = vec![vec![0.0f64; m]; m];
        for (a, &i) in free.iter().enumerate() {
            for j in 0..n {
                if g[i][j] <= 0.0 {
                    continue;
                }
                let p = expected(r[i], r[j]);
                grad[a] += K * (s[i][j] - g[i][j] * p);
                let w = K * K * g[i][j] * p * (1.0 - p);
                hess[a][a] += w;
                if let Some(b) = free.iter().position(|&x| x == j) {
                    hess[a][b] -= w;
                }
            }
        }
        // Solve hess * step = grad (hess is the negated Hessian: symmetric positive definite).
        let Some(step) = solve(hess, grad) else { break };
        let mut biggest = 0.0f64;
        for (a, &i) in free.iter().enumerate() {
            let d = step[a].clamp(-400.0, 400.0);
            r[i] += d;
            biggest = biggest.max(d.abs());
        }
        if biggest < 1e-6 {
            break;
        }
    }
    let mean = active.iter().map(|&i| r[i]).sum::<f64>() / active.len() as f64;
    for &i in &active {
        r[i] -= mean;
    }
    r
}

/// Gaussian elimination with partial pivoting. `None` if the system is singular.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let piv = (col..n).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for row in col + 1..n {
            let f = a[row][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            let pivot_row = a[col].clone();
            for (dst, src) in a[row].iter_mut().zip(&pivot_row).skip(col) {
                *dst -= f * src;
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.0f64; n];
    for row in (0..n).rev() {
        let mut acc = b[row];
        for k in row + 1..n {
            acc -= a[row][k] * x[k];
        }
        x[row] = acc / a[row][row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Shift `ratings` so that the mean over `anchors` (index, target rating) equals the mean of the
/// targets. With the ladder bots' labels as targets this keeps the scale comparable to the labels
/// while letting every individual bot land wherever the games put it.
pub fn anchor_to_mean(ratings: &mut [f64], anchors: &[(usize, f64)]) {
    let valid: Vec<&(usize, f64)> = anchors.iter().filter(|(i, _)| *i < ratings.len()).collect();
    if valid.is_empty() {
        return;
    }
    let n = valid.len() as f64;
    let have = valid.iter().map(|(i, _)| ratings[*i]).sum::<f64>() / n;
    let want = valid.iter().map(|(_, t)| *t).sum::<f64>() / n;
    let shift = want - have;
    ratings.iter_mut().for_each(|r| *r += shift);
}

/// Fit + anchor, then a bootstrap over game pairs. Returns (ratings, per-player (lo, hi) of the
/// central `level` interval, e.g. 0.95).
pub fn fit_with_ci(
    n: usize,
    games: &[GameResult],
    anchors: &[(usize, f64)],
    resamples: usize,
    level: f64,
    seed: u64,
) -> (Vec<f64>, Vec<(f64, f64)>) {
    let mut point = fit(n, games);
    anchor_to_mean(&mut point, anchors);
    // Group games by pair id.
    let mut ids: Vec<usize> = games.iter().map(|g| g.pair).collect();
    ids.sort_unstable();
    ids.dedup();
    let groups: Vec<Vec<GameResult>> =
        ids.iter().map(|id| games.iter().filter(|g| g.pair == *id).copied().collect()).collect();
    let mut rng = StdRng::seed_from_u64(seed);
    let mut samples: Vec<Vec<f64>> = vec![Vec::with_capacity(resamples); n];
    if !groups.is_empty() {
        for _ in 0..resamples {
            let mut sample = Vec::with_capacity(games.len());
            for _ in 0..groups.len() {
                sample.extend_from_slice(&groups[rng.gen_range(0..groups.len())]);
            }
            let mut r = fit(n, &sample);
            anchor_to_mean(&mut r, anchors);
            for (i, v) in r.into_iter().enumerate() {
                samples[i].push(v);
            }
        }
    }
    let tail = (1.0 - level.clamp(0.0, 1.0)) / 2.0;
    let ci = samples
        .into_iter()
        .zip(&point)
        .map(|(mut v, &p)| {
            if v.is_empty() {
                return (p, p);
            }
            v.sort_by(f64::total_cmp);
            (percentile(&v, tail), percentile(&v, 1.0 - tail))
        })
        .collect();
    (point, ci)
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulated round-robin between players of known strength: the fit recovers the gaps.
    #[test]
    fn fit_recovers_known_ratings() {
        let truth = [0.0, 150.0, 300.0, 600.0];
        let mut rng = StdRng::seed_from_u64(7);
        let mut games = Vec::new();
        let mut pair = 0;
        for i in 0..truth.len() {
            for j in i + 1..truth.len() {
                for _ in 0..400 {
                    let p = expected(truth[i], truth[j]);
                    let s = if rng.gen::<f64>() < p { 1.0 } else { 0.0 };
                    games.push(GameResult { white: i, black: j, score: s, pair });
                    pair += 1;
                }
            }
        }
        let anchors: Vec<(usize, f64)> = truth.iter().copied().enumerate().collect();
        let (r, ci) = fit_with_ci(truth.len(), &games, &anchors, 100, 0.95, 1);
        for i in 0..truth.len() {
            assert!((r[i] - truth[i]).abs() < 45.0, "player {i}: {} vs {}", r[i], truth[i]);
            assert!(ci[i].0 <= r[i] && r[i] <= ci[i].1);
        }
    }

    #[test]
    fn perfect_scores_stay_finite() {
        let games: Vec<GameResult> =
            (0..20).map(|k| GameResult { white: k % 2, black: 1 - k % 2, score: if k % 2 == 0 { 1.0 } else { 0.0 }, pair: k }).collect();
        let r = fit(2, &games);
        assert!(r.iter().all(|v| v.is_finite()));
        assert!(r[0] > r[1] + 300.0);
        assert!(fit(0, &[]).is_empty());
        assert_eq!(fit(3, &[]), vec![0.0; 3]);
    }
}
