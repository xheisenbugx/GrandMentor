//! Puzzle selection: rating-window search with theme filter, recent-avoidance, the daily
//! puzzle (deterministic by date) and Puzzle Rush sets.

use std::collections::{BTreeMap, HashSet};

use gm_content::{Content, Puzzle};
use rand::seq::SliceRandom;
use rand::Rng;
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ThemeCount {
    pub theme: String,
    pub count: u32,
}

/// Precomputed, immutable view over `content.puzzles`.
pub struct PuzzleIndex {
    /// Indices into `content.puzzles`, sorted by (rating, id).
    by_rating: Vec<usize>,
    /// Sorted by count desc, then name.
    pub themes: Vec<ThemeCount>,
}

/// True for theme filters that mean "any theme".
fn is_any_theme(theme: Option<&str>) -> bool {
    match theme.map(str::trim) {
        None => true,
        Some(t) => {
            t.is_empty()
                || t.eq_ignore_ascii_case("all")
                || t.eq_ignore_ascii_case("mix")
                || t.eq_ignore_ascii_case("any")
        }
    }
}

fn has_theme(p: &Puzzle, theme: &str) -> bool {
    p.themes.iter().any(|t| t.eq_ignore_ascii_case(theme))
}

impl PuzzleIndex {
    pub fn build(content: &Content) -> Self {
        let mut by_rating: Vec<usize> = (0..content.puzzles.len()).collect();
        by_rating.sort_by(|&a, &b| {
            let (pa, pb) = (&content.puzzles[a], &content.puzzles[b]);
            pa.rating.cmp(&pb.rating).then_with(|| pa.id.cmp(&pb.id))
        });
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for p in &content.puzzles {
            let mut seen = HashSet::new();
            for t in &p.themes {
                let t = t.trim();
                if !t.is_empty() && seen.insert(t.to_string()) {
                    *counts.entry(t.to_string()).or_default() += 1;
                }
            }
        }
        let mut themes: Vec<ThemeCount> = counts
            .into_iter()
            .map(|(theme, count)| ThemeCount { theme, count })
            .collect();
        themes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.theme.cmp(&b.theme)));
        PuzzleIndex { by_rating, themes }
    }

    pub fn len(&self) -> usize {
        self.by_rating.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_rating.is_empty()
    }

    /// Indices (sorted by rating) whose rating is in `[min, max]`.
    fn range<'a>(&'a self, content: &'a Content, min: u16, max: u16) -> &'a [usize] {
        let lo = self
            .by_rating
            .partition_point(|&i| content.puzzles[i].rating < min);
        let hi = self
            .by_rating
            .partition_point(|&i| content.puzzles[i].rating <= max);
        if lo >= hi {
            &[]
        } else {
            &self.by_rating[lo..hi]
        }
    }

    /// Pick a puzzle near `target` within `[min, max]` (widening the window when empty),
    /// matching `theme`, avoiding `recent` where possible.
    pub fn select_next<'a>(
        &self,
        content: &'a Content,
        target: u16,
        min: u16,
        max: u16,
        theme: Option<&str>,
        recent: &HashSet<String>,
    ) -> Option<&'a Puzzle> {
        if self.by_rating.is_empty() {
            return None;
        }
        let any = is_any_theme(theme);
        let theme = theme.map(str::trim).unwrap_or("");
        let matches = |i: &usize| any || has_theme(&content.puzzles[*i], theme);
        let (min, max) = if min <= max { (min, max) } else { (max, min) };
        let mut rng = rand::thread_rng();

        // Widen the window progressively: requested, ±300, ±700, everything.
        let windows = [
            (min, max),
            (min.saturating_sub(300), max.saturating_add(300)),
            (min.saturating_sub(700), max.saturating_add(700)),
            (0, u16::MAX),
        ];
        for &(lo, hi) in &windows {
            let fresh: Vec<usize> = self
                .range(content, lo, hi)
                .iter()
                .filter(|i| matches(i) && !recent.contains(&content.puzzles[**i].id))
                .copied()
                .collect();
            if !fresh.is_empty() {
                // Prefer puzzles close to the target: sample from the nearest half (min 8).
                let mut fresh = fresh;
                fresh.sort_by_key(|&i| {
                    (i32::from(content.puzzles[i].rating) - i32::from(target)).unsigned_abs()
                });
                let keep = (fresh.len() / 2).max(8).min(fresh.len());
                let pick = fresh[rng.gen_range(0..keep)];
                return content.puzzles.get(pick);
            }
        }
        // Everything matching was seen recently: allow repeats, nearest to target.
        let all: Vec<usize> = self
            .by_rating
            .iter()
            .filter(|i| matches(i))
            .copied()
            .collect();
        let chosen = all.choose(&mut rng).copied()?;
        content.puzzles.get(chosen)
    }

    /// Deterministic puzzle of the day for `day` (days since Unix epoch, UTC).
    /// Prefers mid-rated puzzles (1000..=2000) so it's approachable for most users.
    pub fn daily<'a>(&self, content: &'a Content, day: u64) -> Option<&'a Puzzle> {
        if self.by_rating.is_empty() {
            return None;
        }
        let mid = self.range(content, 1000, 2000);
        let pool = if mid.is_empty() {
            &self.by_rating[..]
        } else {
            mid
        };
        // splitmix64 so consecutive days don't map to neighbouring (similar-rated) puzzles.
        let mut z = day.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let idx = (z % pool.len() as u64) as usize;
        content.puzzles.get(pool[idx])
    }

    /// `count` puzzles of increasing difficulty: the rating-sorted list is split into
    /// `count` buckets and one random puzzle is taken from each.
    pub fn rush<'a>(&self, content: &'a Content, count: usize) -> Vec<&'a Puzzle> {
        let n = self.by_rating.len();
        if n == 0 || count == 0 {
            return Vec::new();
        }
        let count = count.min(n);
        let mut rng = rand::thread_rng();
        let mut out = Vec::with_capacity(count);
        for b in 0..count {
            let lo = b * n / count;
            let hi = ((b + 1) * n / count).max(lo + 1).min(n);
            let pick = self.by_rating[rng.gen_range(lo..hi)];
            if let Some(p) = content.puzzles.get(pick) {
                out.push(p);
            }
        }
        out.sort_by(|a, b| a.rating.cmp(&b.rating).then_with(|| a.id.cmp(&b.id)));
        out
    }
}

/// Days since the Unix epoch (UTC) for "now".
pub fn today_utc() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pz(id: &str, rating: u16, themes: &[&str]) -> Puzzle {
        Puzzle {
            id: id.into(),
            fen: String::new(),
            moves: vec![],
            rating,
            themes: themes.iter().map(|s| s.to_string()).collect(),
            popularity: 0,
        }
    }

    fn content() -> Content {
        Content {
            puzzles: vec![
                pz("a", 800, &["fork"]),
                pz("b", 1200, &["pin", "short"]),
                pz("c", 1500, &["fork", "short"]),
                pz("d", 1900, &["mateIn2"]),
                pz("e", 2400, &["fork"]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn themes_counted() {
        let c = content();
        let idx = PuzzleIndex::build(&c);
        assert_eq!(
            idx.themes[0],
            ThemeCount {
                theme: "fork".into(),
                count: 3
            }
        );
        assert_eq!(
            idx.themes[1],
            ThemeCount {
                theme: "short".into(),
                count: 2
            }
        );
    }

    #[test]
    fn next_respects_window_theme_and_recent() {
        let c = content();
        let idx = PuzzleIndex::build(&c);
        let none = HashSet::new();
        let p = idx
            .select_next(&c, 1500, 1400, 1600, None, &none)
            .map(|p| p.id.as_str());
        assert_eq!(p, Some("c"));
        let p = idx
            .select_next(&c, 1200, 1000, 1300, Some("FORK"), &none)
            .map(|p| p.id.as_str());
        // window widens to find a fork puzzle
        assert!(matches!(p, Some("a") | Some("c")));
        let recent: HashSet<String> = ["c".to_string()].into();
        let p = idx
            .select_next(&c, 1500, 1400, 1600, None, &recent)
            .map(|p| p.id.clone());
        assert_ne!(p.as_deref(), Some("c"));
        // all seen -> still returns something
        let all: HashSet<String> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(idx.select_next(&c, 1500, 1400, 1600, None, &all).is_some());
        assert!(idx
            .select_next(&c, 1500, 0, 3000, Some("nonexistent"), &none)
            .is_none());
    }

    #[test]
    fn daily_is_deterministic() {
        let c = content();
        let idx = PuzzleIndex::build(&c);
        let a = idx.daily(&c, 20_000).map(|p| p.id.clone());
        let b = idx.daily(&c, 20_000).map(|p| p.id.clone());
        assert_eq!(a, b);
        let r = idx.daily(&c, 20_000).map(|p| p.rating).unwrap_or(0);
        assert!((1000..=2000).contains(&r));
    }

    #[test]
    fn rush_ascending() {
        let c = content();
        let idx = PuzzleIndex::build(&c);
        let r = idx.rush(&c, 40);
        assert_eq!(r.len(), 5);
        assert!(r.windows(2).all(|w| w[0].rating <= w[1].rating));
        assert_eq!(idx.rush(&c, 3).len(), 3);
        assert!(PuzzleIndex::build(&Content::default())
            .rush(&Content::default(), 10)
            .is_empty());
    }
}
