//! Pure logic of the weekly personal set: ISO weeks, theme weakness ranking, classifying the
//! user's own mistakes by tactic theme and picking the puzzles. Everything here is
//! deterministic for a given input and seed (see the tests).

use std::collections::{HashMap, HashSet};

use gm_content::Puzzle;
use gm_engine::Score;
use gm_store::weekly::MistakePosition;
use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Position};

/// Tactic themes the weekly set can focus on, in the order a beginner should meet them (also
/// the tie-break and the fallback when there is no data yet).
pub const TRACKED: &[&str] = &[
    "hangingPiece",
    "fork",
    "mateIn1",
    "pin",
    "backRankMate",
    "skewer",
    "discoveredAttack",
    "mateIn2",
    "trappedPiece",
    "deflection",
    "attraction",
    "promotion",
    "sacrifice",
    "doubleCheck",
    "mateIn3",
    "clearance",
    "intermezzo",
    "xRayAttack",
    "capturingDefender",
    "discoveredCheck",
    "defensiveMove",
    "quietMove",
];

/// Items in a set.
pub const SET_SIZE: usize = 12;
/// At most this many items come from the user's own games.
pub const MAX_OWN: usize = 4;
/// Focus themes per set.
pub const MAX_FOCUS: usize = 3;
/// Pack puzzles of a theme needed before it can be a focus theme.
pub const MIN_THEME_PUZZLES: usize = 6;
/// Days of history that count for the ranking.
pub const WINDOW_DAYS: f64 = 60.0;
/// Days counted as "this month" in the reasons shown to the user.
pub const MONTH_DAYS: f64 = 30.0;
/// Recency half-life of an observation, in days.
const HALF_LIFE_DAYS: f64 = 21.0;
/// Pseudo-observations pulling a theme's fail rate towards the user's overall fail rate.
const PRIOR_WEIGHT: f64 = 4.0;
/// Fail rate assumed when the user has no puzzle history.
const DEFAULT_FAIL_RATE: f64 = 0.35;
/// Weight of one recent missed chance in a real game.
const GAME_MISS_WEIGHT: f64 = 0.35;
const GAME_MISS_CAP: f64 = 1.4;

// ---------------------------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------------------------

/// `(year, month, day)` of a day number (days since 1970-01-01, UTC).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Day number of a civil date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn ymd(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// An ISO-8601 week: key `YYYY-Www` and its Monday (day number).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoWeek {
    pub key: String,
    pub monday: i64,
}

impl IsoWeek {
    pub fn of_day(days: i64) -> IsoWeek {
        // 1970-01-01 was a Thursday; 0 = Monday.
        let weekday = (days + 3).rem_euclid(7);
        let monday = days - weekday;
        let thursday = monday + 3;
        let (year, _, _) = civil_from_days(thursday);
        let week = (thursday - days_from_civil(year, 1, 1)) / 7 + 1;
        IsoWeek { key: format!("{year:04}-W{week:02}"), monday }
    }

    /// The week `n` weeks before this one.
    pub fn back(&self, n: i64) -> IsoWeek {
        IsoWeek::of_day(self.monday - 7 * n)
    }

    pub fn start(&self) -> String {
        ymd(self.monday)
    }

    pub fn end(&self) -> String {
        ymd(self.monday + 6)
    }
}

// ---------------------------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------------------------

/// A rated puzzle attempt with the puzzle's themes.
#[derive(Clone, Debug)]
pub struct PuzzleObs {
    pub themes: Vec<String>,
    pub solved: bool,
    pub age_days: f64,
}

/// A missed tactic in one of the user's games.
#[derive(Clone, Debug)]
pub struct GameObs {
    pub theme: String,
    pub age_days: f64,
}

/// Why a theme was picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    /// The user missed this tactic in their own games recently.
    Games,
    /// The user often fails puzzles of this theme.
    Puzzles,
    /// No evidence yet: a key pattern for every player.
    Starter,
}

/// One ranked theme (sent to the UI, stored with the set).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FocusTheme {
    pub theme: String,
    pub reason: Reason,
    /// Missed chances of this theme in reviewed games during the last 30 days.
    pub game_misses: u32,
    /// Rated puzzle attempts / failures of this theme in the last 30 days.
    pub puzzle_attempts: u32,
    pub puzzle_fails: u32,
    /// Weakness score (higher = weaker). Rounded to 3 decimals.
    pub score: f64,
}

fn weight(age_days: f64) -> f64 {
    if !age_days.is_finite() || age_days > WINDOW_DAYS {
        return 0.0;
    }
    0.5f64.powf(age_days.max(0.0) / HALF_LIFE_DAYS)
}

/// Ranks every tracked theme that `available` accepts, weakest first.
///
/// * Puzzles: recency-weighted fail rate per theme, smoothed towards the user's overall fail
///   rate, minus that overall rate (so only relative weakness counts).
/// * Games: each recent missed chance adds [`GAME_MISS_WEIGHT`] (capped).
/// * Ties (and no data at all) fall back to the beginner order of [`TRACKED`].
pub fn rank_themes(puzzles: &[PuzzleObs], games: &[GameObs], available: impl Fn(&str) -> bool) -> Vec<FocusTheme> {
    let tracked: HashSet<&str> = TRACKED.iter().copied().collect();
    let (mut fail_w, mut total_w) = (0.0, 0.0);
    #[derive(Default)]
    struct Acc {
        fail_w: f64,
        total_w: f64,
        attempts: u32,
        fails: u32,
        miss_w: f64,
        misses: u32,
    }
    let mut acc: HashMap<&str, Acc> = HashMap::new();
    for p in puzzles {
        let w = weight(p.age_days);
        if w <= 0.0 {
            continue;
        }
        total_w += w;
        if !p.solved {
            fail_w += w;
        }
        let mut seen = HashSet::new();
        for t in &p.themes {
            let Some(&t) = tracked.get(t.as_str()) else { continue };
            if !seen.insert(t) {
                continue;
            }
            let a = acc.entry(t).or_default();
            a.total_w += w;
            if !p.solved {
                a.fail_w += w;
            }
            if p.age_days <= MONTH_DAYS {
                a.attempts += 1;
                a.fails += u32::from(!p.solved);
            }
        }
    }
    for g in games {
        let w = weight(g.age_days);
        let Some(&t) = tracked.get(g.theme.as_str()) else { continue };
        if w <= 0.0 {
            continue;
        }
        let a = acc.entry(t).or_default();
        a.miss_w += w;
        if g.age_days <= MONTH_DAYS {
            a.misses += 1;
        }
    }
    let base = if total_w > 0.0 { fail_w / total_w } else { DEFAULT_FAIL_RATE };
    let n = TRACKED.len() as f64;
    let mut out: Vec<(usize, FocusTheme)> = TRACKED
        .iter()
        .enumerate()
        .filter(|(_, t)| available(t))
        .map(|(i, &t)| {
            let a = acc.remove(t).unwrap_or_default();
            let puzzle_part = if a.total_w > 0.0 {
                2.0 * ((a.fail_w + PRIOR_WEIGHT * base) / (a.total_w + PRIOR_WEIGHT) - base)
            } else {
                0.0
            };
            let game_part = (GAME_MISS_WEIGHT * a.miss_w).min(GAME_MISS_CAP);
            let order = 0.01 * (n - i as f64) / n;
            let score = puzzle_part + game_part + order;
            let reason = if a.misses > 0 && game_part >= puzzle_part {
                Reason::Games
            } else if a.fails > 0 && puzzle_part > 0.0 {
                Reason::Puzzles
            } else if a.misses > 0 {
                Reason::Games
            } else {
                Reason::Starter
            };
            let ft = FocusTheme {
                theme: t.to_string(),
                reason,
                game_misses: a.misses,
                puzzle_attempts: a.attempts,
                puzzle_fails: a.fails,
                score: (score * 1000.0).round() / 1000.0,
            };
            (i, ft)
        })
        .collect();
    out.sort_by(|(ia, a), (ib, b)| b.score.total_cmp(&a.score).then(ia.cmp(ib)));
    out.into_iter().map(|(_, f)| f).collect()
}

/// The focus themes of a set: the 2–3 weakest. Two when exactly two themes have evidence (the
/// set stays on what matters), otherwise three.
pub fn pick_focus(ranked: &[FocusTheme]) -> Vec<FocusTheme> {
    let evidence = ranked.iter().take(MAX_FOCUS).filter(|f| f.reason != Reason::Starter).count();
    let n = if evidence == 2 { 2 } else { MAX_FOCUS };
    ranked.iter().take(n).cloned().collect()
}

// ---------------------------------------------------------------------------------------------
// Classifying own mistakes
// ---------------------------------------------------------------------------------------------

/// The tactic theme of a mistake card: what the best move would have done (a mate, fork, pin,
/// ...) or, failing that, `hangingPiece` when the played move left a piece en prise.
pub fn classify_mistake(m: &MistakePosition) -> Option<&'static str> {
    let pos: Chess = gm_engine::parse_fen(&m.fen).ok()?;
    let best = gm_engine::uci_to_move(&pos, &m.best_uci).ok()?;
    // A solution line ending in mate tells us the mate length.
    let mut eval = Score::Cp(0);
    if m.solution.len() >= 3 && m.solution.len() % 2 == 1 && m.solution.first() == Some(&m.best_uci) {
        let mut p = pos.clone();
        let mut ok = true;
        for u in &m.solution {
            match gm_engine::uci_to_move(&p, u) {
                Ok(mv) => p.play_unchecked(&mv),
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && p.is_checkmate() {
            let n = m.solution.len().div_ceil(2) as i32;
            eval = Score::Mate(if pos.turn().is_white() { n } else { -n });
        }
    }
    if let Some(t) = gm_analysis::insights::tactic_theme(&pos, &best, eval) {
        return Some(t);
    }
    let played = gm_engine::uci_to_move(&pos, &m.played_uci).ok()?;
    gm_analysis::insights::hung_piece(&pos, &played).map(|_| "hangingPiece")
}

// ---------------------------------------------------------------------------------------------
// Building a set
// ---------------------------------------------------------------------------------------------

/// SplitMix64: tiny, portable and deterministic.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform-ish in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Seed of a set: FNV-1a of the week key mixed with the generation.
pub fn seed_for(week: &str, generation: u32) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in week.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h ^ (u64::from(generation).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// A mistake card with its theme.
#[derive(Clone, Debug)]
pub struct OwnCandidate {
    pub card_id: i64,
    pub theme: Option<String>,
    pub age_days: f64,
    pub win_chance_loss: f32,
    pub graduated: bool,
}

/// One picked item.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// Index into the puzzle slice, and the focus theme it trains.
    Puzzle { index: usize, theme: String },
    /// A mistake card, and its theme when it has one.
    Own { card_id: i64, theme: Option<String> },
}

pub struct SetInput<'a> {
    pub puzzles: &'a [Puzzle],
    pub rating: u16,
    pub focus: &'a [FocusTheme],
    /// Puzzle ids attempted recently (avoided when possible).
    pub avoid: &'a HashSet<String>,
    pub own: &'a [OwnCandidate],
    pub seed: u64,
}

/// Up to [`MAX_OWN`] own-game positions: focus-theme ones first, then still-learning, recent
/// and costly ones, with a little seeded jitter so "New set" brings different positions.
fn pick_own(input: &SetInput<'_>, rng: &mut Rng) -> Vec<Pick> {
    let focus: HashSet<&str> = input.focus.iter().map(|f| f.theme.as_str()).collect();
    let mut scored: Vec<(f64, &OwnCandidate)> = input
        .own
        .iter()
        .map(|c| {
            let in_focus = c.theme.as_deref().is_some_and(|t| focus.contains(t));
            let s = 2.0 * f64::from(u8::from(in_focus))
                + 0.6 * f64::from(u8::from(c.theme.is_some()))
                + f64::from(u8::from(!c.graduated))
                + (1.0 - (c.age_days / WINDOW_DAYS).clamp(0.0, 1.0))
                + f64::from(c.win_chance_loss.clamp(0.0, 100.0)) / 100.0
                + 0.8 * rng.unit();
            (s, c)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.card_id.cmp(&b.1.card_id)));
    scored
        .into_iter()
        .take(MAX_OWN)
        .map(|(_, c)| Pick::Own { card_id: c.card_id, theme: c.theme.clone() })
        .collect()
}

fn has_theme(p: &Puzzle, theme: &str) -> bool {
    p.themes.iter().any(|t| t == theme)
}

/// Picks one puzzle of `theme` near `target` (sampling among the nearest few).
fn pick_near(
    input: &SetInput<'_>,
    theme: Option<&str>,
    target: i32,
    taken: &HashSet<usize>,
    rng: &mut Rng,
) -> Option<usize> {
    for allow_recent in [false, true] {
        let mut cands: Vec<(u32, usize)> = input
            .puzzles
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                !taken.contains(i)
                    && theme.is_none_or(|t| has_theme(p, t))
                    && (allow_recent || !input.avoid.contains(&p.id))
            })
            .map(|(i, p)| ((i32::from(p.rating) - target).unsigned_abs(), i))
            .collect();
        if cands.is_empty() {
            continue;
        }
        cands.sort_by(|a, b| a.0.cmp(&b.0).then(input.puzzles[a.1].id.cmp(&input.puzzles[b.1].id)));
        let keep = cands.len().min(6);
        return Some(cands[rng.below(keep)].1);
    }
    None
}

/// Builds the set: mostly pack puzzles from the focus themes near the user's rating (gently
/// ramping up in difficulty), with own-game positions slotted in every third item.
pub fn build_set(input: &SetInput<'_>) -> Vec<Pick> {
    let mut rng = Rng::new(input.seed);
    let own = pick_own(input, &mut rng);
    let n_pack = SET_SIZE.saturating_sub(own.len());

    // Round-robin quotas over the focus themes.
    let themes: Vec<&str> = input.focus.iter().map(|f| f.theme.as_str()).collect();
    let mut quota: Vec<usize> = vec![0; themes.len()];
    for i in 0..n_pack {
        if !themes.is_empty() {
            quota[i % themes.len()] += 1;
        }
    }
    let rating = i32::from(input.rating);
    let mut taken: HashSet<usize> = HashSet::new();
    let mut pack: Vec<Pick> = Vec::new();
    for (ti, &theme) in themes.iter().enumerate() {
        let q = quota[ti];
        for k in 0..q {
            // -100 .. +150 around the rating.
            let target = rating - 100 + if q > 1 { (250 * k as i32) / (q as i32 - 1) } else { 0 };
            if let Some(i) = pick_near(input, Some(theme), target, &taken, &mut rng) {
                taken.insert(i);
                pack.push(Pick::Puzzle { index: i, theme: theme.to_string() });
            }
        }
    }
    // Top up from any theme when a focus theme ran dry.
    while pack.len() < n_pack {
        let Some(i) = pick_near(input, None, rating, &taken, &mut rng) else { break };
        taken.insert(i);
        let theme = input.puzzles[i]
            .themes
            .iter()
            .find(|t| themes.contains(&t.as_str()))
            .cloned()
            .unwrap_or_default();
        pack.push(Pick::Puzzle { index: i, theme });
    }
    pack.sort_by_key(|p| match p {
        Pick::Puzzle { index, .. } => (input.puzzles[*index].rating, input.puzzles[*index].id.clone()),
        Pick::Own { .. } => (0, String::new()),
    });

    // Interleave: own positions at slots 2, 5, 8, 11 (0-based), pack puzzles elsewhere.
    let mut out = Vec::with_capacity(pack.len() + own.len());
    let mut own = own.into_iter();
    let mut pack = pack.into_iter();
    loop {
        let next = if out.len() % 3 == 2 { own.next().or_else(|| pack.next()) } else { pack.next().or_else(|| own.next()) };
        match next {
            Some(p) => out.push(p),
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pz(id: &str, rating: u16, themes: &[&str]) -> Puzzle {
        Puzzle {
            id: id.into(),
            fen: "8/8/8/8/8/8/8/K6k w - - 0 1".into(),
            moves: vec!["a1a2".into(), "h1h2".into()],
            rating,
            themes: themes.iter().map(|s| s.to_string()).collect(),
            popularity: 90,
        }
    }

    fn obs(theme: &str, solved: bool, age: f64) -> PuzzleObs {
        PuzzleObs { themes: vec![theme.into(), "middlegame".into()], solved, age_days: age }
    }

    #[test]
    fn iso_weeks() {
        let d = |y, m, dd| days_from_civil(y, m, dd);
        assert_eq!(civil_from_days(d(2026, 10, 8)), (2026, 10, 8));
        assert_eq!(IsoWeek::of_day(d(2026, 10, 8)).key, "2026-W41");
        assert_eq!(IsoWeek::of_day(d(2026, 10, 8)).start(), "2026-10-05");
        assert_eq!(IsoWeek::of_day(d(2026, 10, 11)).key, "2026-W41");
        assert_eq!(IsoWeek::of_day(d(2026, 10, 12)).key, "2026-W42");
        // Year boundaries.
        assert_eq!(IsoWeek::of_day(d(2021, 1, 3)).key, "2020-W53");
        assert_eq!(IsoWeek::of_day(d(2024, 12, 30)).key, "2025-W01");
        assert_eq!(IsoWeek::of_day(d(2026, 1, 1)).key, "2026-W01");
        assert_eq!(IsoWeek::of_day(d(2026, 10, 8)).back(1).key, "2026-W40");
        assert_eq!(IsoWeek::of_day(d(2026, 1, 1)).back(1).key, "2025-W52");
        for day in d(2000, 1, 1)..d(2040, 1, 1) {
            assert!(gm_store::weekly::valid_week(&IsoWeek::of_day(day).key));
        }
    }

    #[test]
    fn ranking_without_data_follows_beginner_order() {
        let r = rank_themes(&[], &[], |_| true);
        assert_eq!(r.len(), TRACKED.len());
        assert_eq!(r.iter().take(3).map(|f| f.theme.as_str()).collect::<Vec<_>>(), ["hangingPiece", "fork", "mateIn1"]);
        assert!(r.iter().all(|f| f.reason == Reason::Starter));
        assert_eq!(pick_focus(&r).len(), 3);
        // Unavailable themes are skipped.
        let r = rank_themes(&[], &[], |t| t != "fork");
        assert_eq!(r[1].theme, "mateIn1");
    }

    #[test]
    fn ranking_uses_puzzle_fail_rate_and_recency() {
        let mut p = Vec::new();
        // Pins: 4 of 5 failed recently. Forks: 1 of 5 failed. Skewers: failed long ago (outside the window).
        for i in 0..5 {
            p.push(obs("pin", i == 0, 2.0));
            p.push(obs("fork", i != 0, 3.0));
            p.push(obs("skewer", false, 90.0));
        }
        let r = rank_themes(&p, &[], |_| true);
        assert_eq!(r[0].theme, "pin");
        assert_eq!(r[0].reason, Reason::Puzzles);
        assert_eq!((r[0].puzzle_attempts, r[0].puzzle_fails), (5, 4));
        let fork = r.iter().position(|f| f.theme == "fork").unwrap();
        let skewer = r.iter().position(|f| f.theme == "skewer").unwrap();
        assert!(fork > 3, "forks are a strength: {fork}");
        assert_eq!(r[skewer].puzzle_attempts, 0, "old attempts do not count");

        // A fresh failure weighs more than an old one.
        let recent = rank_themes(&[obs("pin", false, 1.0), obs("fork", false, 50.0), obs("pin", true, 1.0), obs("fork", true, 50.0), obs("mateIn1", true, 1.0)], &[], |_| true);
        let pin = recent.iter().find(|f| f.theme == "pin").unwrap().score;
        let fork = recent.iter().find(|f| f.theme == "fork").unwrap().score;
        assert!(pin > fork - 0.01, "{pin} vs {fork}");
    }

    #[test]
    fn ranking_uses_game_misses() {
        let games: Vec<GameObs> = (0..4).map(|i| GameObs { theme: "fork".into(), age_days: i as f64 * 5.0 }).collect();
        let mut more = games.clone();
        more.push(GameObs { theme: "backRankMate".into(), age_days: 3.0 });
        more.push(GameObs { theme: "backRankMate".into(), age_days: 45.0 });
        let r = rank_themes(&[obs("pin", false, 1.0), obs("pin", true, 1.0)], &more, |_| true);
        assert_eq!(r[0].theme, "fork");
        assert_eq!((r[0].reason, r[0].game_misses), (Reason::Games, 4));
        assert_eq!(r[1].theme, "backRankMate");
        assert_eq!(r[1].game_misses, 1, "only the last 30 days are reported");
        let focus = pick_focus(&r);
        assert_eq!(focus.len(), 2, "two themes with evidence: {focus:?}");
        // Deterministic.
        assert_eq!(r, rank_themes(&[obs("pin", false, 1.0), obs("pin", true, 1.0)], &more, |_| true));
    }

    fn pack() -> Vec<Puzzle> {
        let mut v = Vec::new();
        for i in 0..40u16 {
            v.push(pz(&format!("f{i:02}"), 800 + i * 25, &["fork", "middlegame"]));
            v.push(pz(&format!("p{i:02}"), 800 + i * 25, &["pin"]));
            v.push(pz(&format!("m{i:02}"), 800 + i * 25, &["mateIn1", "fork"]));
        }
        v.push(pz("s1", 1200, &["skewer"]));
        v
    }

    fn focus(themes: &[&str]) -> Vec<FocusTheme> {
        themes
            .iter()
            .map(|t| FocusTheme { theme: t.to_string(), reason: Reason::Starter, game_misses: 0, puzzle_attempts: 0, puzzle_fails: 0, score: 0.0 })
            .collect()
    }

    #[test]
    fn set_is_deterministic_and_balanced() {
        let puzzles = pack();
        let f = focus(&["fork", "pin", "mateIn1"]);
        let avoid: HashSet<String> = ["f16".to_string(), "f17".to_string()].into_iter().collect();
        let own: Vec<OwnCandidate> = (1..=6)
            .map(|i| OwnCandidate {
                card_id: i,
                theme: if i % 2 == 0 { Some("pin".into()) } else { None },
                age_days: i as f64,
                win_chance_loss: 20.0,
                graduated: i == 2,
            })
            .collect();
        let input = SetInput { puzzles: &puzzles, rating: 1200, focus: &f, avoid: &avoid, own: &own, seed: seed_for("2026-W41", 1) };
        let a = build_set(&input);
        assert_eq!(a, build_set(&input), "same seed, same set");
        assert_eq!(a.len(), SET_SIZE);

        let owns: Vec<i64> = a.iter().filter_map(|p| match p { Pick::Own { card_id, .. } => Some(*card_id), _ => None }).collect();
        assert_eq!(owns.len(), MAX_OWN);
        assert!(owns.contains(&4) && owns.contains(&6), "focus-theme positions first: {owns:?}");
        for (i, p) in a.iter().enumerate() {
            assert_eq!(matches!(p, Pick::Own { .. }), i % 3 == 2, "slot {i}: {p:?}");
        }
        let mut per_theme: HashMap<&str, usize> = HashMap::new();
        let mut ids = HashSet::new();
        for p in &a {
            if let Pick::Puzzle { index, theme } = p {
                let pz = &puzzles[*index];
                assert!(has_theme(pz, theme), "{} has {theme}", pz.id);
                assert!(!avoid.contains(&pz.id), "recent puzzle {} avoided", pz.id);
                assert!((1000..=1450).contains(&pz.rating), "near the rating: {}", pz.rating);
                assert!(ids.insert(pz.id.clone()), "no duplicates");
                *per_theme.entry(theme.as_str()).or_default() += 1;
            }
        }
        // 8 pack puzzles over 3 themes, round robin.
        let mut counts: Vec<usize> = per_theme.values().copied().collect();
        counts.sort_unstable();
        assert_eq!(counts, [2, 3, 3]);
        // Pack puzzles ramp up in difficulty.
        let ratings: Vec<u16> = a.iter().filter_map(|p| match p { Pick::Puzzle { index, .. } => Some(puzzles[*index].rating), _ => None }).collect();
        assert!(ratings.windows(2).all(|w| w[0] <= w[1]), "{ratings:?}");

        // Another generation gives a different set.
        let b = build_set(&SetInput { seed: seed_for("2026-W41", 2), ..input });
        assert_ne!(a, b);
    }

    #[test]
    fn set_tops_up_when_a_theme_runs_dry() {
        let puzzles = pack();
        let f = focus(&["skewer", "pin"]);
        let avoid = HashSet::new();
        let a = build_set(&SetInput { puzzles: &puzzles, rating: 1200, focus: &f, avoid: &avoid, own: &[], seed: 7 });
        assert_eq!(a.len(), SET_SIZE);
        let skewers = a.iter().filter(|p| matches!(p, Pick::Puzzle { theme, .. } if theme == "skewer")).count();
        assert_eq!(skewers, 1, "only one skewer puzzle exists");
        // Empty pack: only own positions.
        let own = vec![OwnCandidate { card_id: 9, theme: None, age_days: 1.0, win_chance_loss: 30.0, graduated: false }];
        let only = build_set(&SetInput { puzzles: &[], rating: 1200, focus: &f, avoid: &avoid, own: &own, seed: 1 });
        assert_eq!(only, vec![Pick::Own { card_id: 9, theme: None }]);
    }

    #[test]
    fn classifies_own_mistakes() {
        let base = MistakePosition {
            id: 1,
            classification: "blunder".into(),
            ..Default::default()
        };
        // Knight fork of king and queen: Nc7+ (white knight b5, black king e8, queen a8).
        let fork = MistakePosition {
            fen: "q3k3/8/8/1N6/8/8/8/4K3 w - - 0 1".into(),
            best_uci: "b5c7".into(),
            played_uci: "e1e2".into(),
            solution: vec!["b5c7".into()],
            ..base.clone()
        };
        assert_eq!(classify_mistake(&fork), Some("fork"));
        // Back-rank mate: Re8#.
        let mate = MistakePosition {
            fen: "6k1/5ppp/8/8/8/8/8/4R1K1 w - - 0 1".into(),
            best_uci: "e1e8".into(),
            played_uci: "g1f1".into(),
            solution: vec!["e1e8".into()],
            ..base.clone()
        };
        assert_eq!(classify_mistake(&mate), Some("backRankMate"));
        // Garbage never panics.
        let bad = MistakePosition { fen: "nonsense".into(), best_uci: "zz".into(), ..base };
        assert_eq!(classify_mistake(&bad), None);
    }
}
