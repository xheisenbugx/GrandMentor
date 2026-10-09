//! Bot rating calibration: plays many fast games between the bots (and fixed-node engine anchors)
//! on all cores, then fits Elo ratings with confidence intervals. See `docs/BOT_CALIBRATION.md`.
//!
//! ```text
//! cargo run --release -p gm-bots --bin gm-calibrate -- [options]
//!   --games N          games per pairing (even; default 80)
//!   --neighbors K      each player meets the next K players up the ladder (default 3)
//!   --threads T        worker threads (default: all cores)
//!   --seed S           master seed (default 1)
//!   --ref-nps N        nodes/second that converts the bots' time limits into node budgets
//!                      (default 1800000, the bench baseline machine)
//!   --max-plies N      adjudicate after this many plies (default 300)
//!   --sparky LIST      adaptive-bot levels to include, default 250,1000,1800,2400,2800 ("" for none)
//!   --anchors LIST     engine anchors as nodes:guess, default 1000:1700,8000:2250,128000:2600
//!   --only LIST        keep only these player ids (quick experiments)
//!   --no-coaches       leave the coach bots out
//!   --no-random        leave the random mover out
//!   --out FILE         summary JSON (default target/calibration/calibration.json)
//!   --games-out FILE   raw games JSON (default target/calibration/games.json)
//!   --fit FILE         skip playing: refit a raw games file written by an earlier run
//!   --bootstrap N      bootstrap resamples for the confidence intervals (default 400)
//!   --data DIR         content directory (default data)
//! ```
//!
//! Every game is deterministic given `--seed` and `--ref-nps`: no wall clock is involved in move
//! choice (time limits become node budgets), so results do not depend on machine load.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::json;
use shakmaty::zobrist::{Zobrist64, ZobristHash};
use shakmaty::{Chess, Color, EnPassantMode, Position};

use gm_bots::calibration::{fit_with_ci, GameResult};
use gm_bots::{choose_move_deterministic, Strength, ADAPTIVE_ID};
use gm_content::{Content, Lang};
use gm_engine::{uci_to_move, Engine, Score, SearchLimits, START_FEN};

const BOT_TT_MB: usize = 32;
const REFEREE_TT_MB: usize = 4;
const REFEREE_NODES: u64 = 6_000;
const OPENING_PLIES: usize = 8;
/// Resign adjudication: referee eval beyond this for `RESIGN_PLIES` plies in a row.
const RESIGN_CP: i32 = 1_000;
const RESIGN_PLIES: usize = 8;
/// Only players at least this strong (nominal) are trusted to convert a won position.
const CONVERTS_FROM: f64 = 1_200.0;
/// Draw adjudication: both players at least this strong, after this ply, eval within the window.
const DRAW_FROM_NOMINAL: f64 = 1_600.0;
const DRAW_FROM_PLY: usize = 120;
const DRAW_CP: i32 = 20;
const DRAW_PLIES: usize = 16;
/// Hard caps on what the command line may ask for (everything stays bounded).
const MAX_GAMES_PER_PAIR: usize = 2_000;
const MAX_PLAYERS: usize = 64;
const MAX_THREADS: usize = 256;
const MAX_ANCHOR_NODES: u64 = 20_000_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Kind {
    /// A persona (`elo` overrides its rating: used for the adaptive bot's levels).
    Bot { bot_id: String, elo: Option<u16> },
    /// The plain engine at a fixed node budget, always playing its best move.
    Engine { nodes: u64 },
    /// Uniformly random legal moves: the bottom of any scale.
    Random,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Player {
    id: String,
    #[serde(flatten)]
    kind: Kind,
    /// The bot's user-facing label (ladder bots and coaches); `None` for Sparky levels/anchors.
    label: Option<u16>,
    /// Part of the main ladder (the fit's anchor set).
    ladder: bool,
    /// Expected strength, only used to order players for pairing and adjudication.
    nominal: f64,
}

#[derive(Clone, Copy, Debug)]
struct Job {
    white: usize,
    black: usize,
    opening: usize,
    pair: usize,
    seed: u64,
    cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Game {
    white: usize,
    black: usize,
    /// White's score: 1, 0.5 or 0.
    score: f64,
    pair: usize,
    plies: usize,
    reason: String,
}

#[derive(Serialize, Deserialize)]
struct RawGames {
    players: Vec<Player>,
    games: Vec<Game>,
    seed: u64,
    ref_nps: u64,
    max_plies: usize,
}

struct Opts {
    games: usize,
    neighbors: usize,
    threads: usize,
    seed: u64,
    ref_nps: u64,
    max_plies: usize,
    sparky: Vec<u16>,
    anchors: Vec<(u64, f64)>,
    only: Option<HashSet<String>>,
    coaches: bool,
    random: bool,
    out: PathBuf,
    games_out: PathBuf,
    fit: Option<PathBuf>,
    bootstrap: usize,
    data: PathBuf,
}

fn parse_list<T: std::str::FromStr>(s: &str) -> Result<Vec<T>, String> {
    s.split(',').map(str::trim).filter(|x| !x.is_empty()).map(|x| x.parse().map_err(|_| format!("bad list item {x:?}"))).collect()
}

fn parse_opts() -> Result<Opts, String> {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let mut o = Opts {
        games: 80,
        neighbors: 3,
        threads: cores,
        seed: 1,
        ref_nps: 1_800_000,
        max_plies: 300,
        sparky: vec![250, 1000, 1800, 2400, 2800],
        anchors: vec![(1_000, 1_700.0), (8_000, 2_250.0), (128_000, 2_600.0)],
        only: None,
        coaches: true,
        random: true,
        out: PathBuf::from("target/calibration/calibration.json"),
        games_out: PathBuf::from("target/calibration/games.json"),
        fit: None,
        bootstrap: 400,
        data: PathBuf::from("data"),
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        if flag == "--no-coaches" {
            o.coaches = false;
            i += 1;
            continue;
        }
        if flag == "--no-random" {
            o.random = false;
            i += 1;
            continue;
        }
        if flag == "-h" || flag == "--help" {
            return Err(String::new());
        }
        let val = args.get(i + 1).ok_or_else(|| format!("{flag} needs a value"))?.as_str();
        let num = |v: &str| v.parse::<u64>().map_err(|_| format!("{flag}: not a number: {v:?}"));
        match flag {
            "--games" => o.games = (num(val)? as usize).clamp(2, MAX_GAMES_PER_PAIR) & !1,
            "--neighbors" => o.neighbors = (num(val)? as usize).clamp(1, MAX_PLAYERS),
            "--threads" => o.threads = (num(val)? as usize).clamp(1, MAX_THREADS),
            "--seed" => o.seed = num(val)?,
            "--ref-nps" => o.ref_nps = num(val)?.clamp(10_000, 1_000_000_000),
            "--max-plies" => o.max_plies = (num(val)? as usize).clamp(40, 1_000),
            "--bootstrap" => o.bootstrap = (num(val)? as usize).min(10_000),
            "--sparky" => o.sparky = parse_list(val)?,
            "--anchors" => {
                o.anchors = val
                    .split(',')
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                    .map(|x| {
                        let (n, g) = x.split_once(':').unwrap_or((x, "1500"));
                        let n: u64 = n.parse().map_err(|_| format!("bad anchor {x:?}"))?;
                        let g: f64 = g.parse().map_err(|_| format!("bad anchor {x:?}"))?;
                        Ok((n.clamp(1, MAX_ANCHOR_NODES), g))
                    })
                    .collect::<Result<_, String>>()?
            }
            "--only" => o.only = Some(parse_list::<String>(val)?.into_iter().collect()),
            "--out" => o.out = PathBuf::from(val),
            "--games-out" => o.games_out = PathBuf::from(val),
            "--fit" => o.fit = Some(PathBuf::from(val)),
            "--data" => o.data = PathBuf::from(val),
            _ => return Err(format!("unknown option {flag}")),
        }
        i += 2;
    }
    Ok(o)
}

fn roster(o: &Opts) -> Vec<Player> {
    let mut out = Vec::new();
    for b in gm_bots::list(Lang::En) {
        if b.id == ADAPTIVE_ID {
            continue;
        }
        let coach = b.category == "coach";
        if coach && !o.coaches {
            continue;
        }
        out.push(Player {
            id: b.id.clone(),
            kind: Kind::Bot { bot_id: b.id.clone(), elo: None },
            label: Some(b.elo),
            ladder: !coach,
            nominal: f64::from(b.elo),
        });
    }
    for &lvl in &o.sparky {
        out.push(Player {
            id: format!("sparky@{lvl}"),
            kind: Kind::Bot { bot_id: ADAPTIVE_ID.into(), elo: Some(lvl) },
            label: None,
            ladder: false,
            nominal: f64::from(lvl),
        });
    }
    for &(nodes, guess) in &o.anchors {
        out.push(Player { id: format!("engine@{nodes}"), kind: Kind::Engine { nodes }, label: None, ladder: false, nominal: guess });
    }
    if o.random {
        out.push(Player { id: "random".into(), kind: Kind::Random, label: None, ladder: false, nominal: -500.0 });
    }
    if let Some(only) = &o.only {
        out.retain(|p| only.contains(&p.id));
    }
    out.truncate(MAX_PLAYERS);
    out.sort_by(|a, b| a.nominal.total_cmp(&b.nominal).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Nodes a player searches per move (for scheduling only).
fn move_cost(p: &Player, ref_nps: u64) -> f64 {
    match &p.kind {
        Kind::Engine { nodes } => *nodes as f64,
        Kind::Random => 0.0,
        Kind::Bot { bot_id, elo } => {
            let e = elo.or_else(|| gm_bots::get(bot_id, Lang::En).map(|b| b.elo)).unwrap_or(1000);
            let st = Strength::for_elo(e);
            st.nodes.min(st.movetime_ms.saturating_mul(ref_nps) / 1000) as f64
        }
    }
}

fn openings(content: &Content) -> Vec<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out: Vec<Vec<String>> = content
        .openings
        .iter()
        .filter(|o| o.uci.len() >= 2)
        .map(|o| o.uci.iter().take(OPENING_PLIES).cloned().collect::<Vec<_>>())
        .filter(|l| seen.insert(l.clone()))
        .collect();
    out.sort();
    out
}

fn mix(a: u64, b: u64) -> u64 {
    // splitmix64 of a combination: decorrelates per-game seeds.
    let mut z = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_add(0xD1B5_4A32_D192_ED03);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn schedule(players: &[Player], o: &Opts, n_openings: usize) -> Vec<Job> {
    let mut jobs = Vec::new();
    let mut pair = 0usize;
    for a in 0..players.len() {
        for b in a + 1..players.len().min(a + 1 + o.neighbors) {
            let mut rng = StdRng::seed_from_u64(mix(o.seed, (a * 1_000 + b) as u64));
            let cost = move_cost(&players[a], o.ref_nps) + move_cost(&players[b], o.ref_nps) + 20_000.0;
            for _ in 0..o.games / 2 {
                let opening = rng.gen_range(0..n_openings.max(1));
                for (w, bl) in [(a, b), (b, a)] {
                    let seed = mix(o.seed ^ 0xC0FFEE, (jobs.len() as u64) << 1 | 1);
                    jobs.push(Job { white: w, black: bl, opening, pair, seed, cost });
                }
                pair += 1;
            }
        }
    }
    jobs
}

fn zobrist(pos: &Chess) -> u64 {
    pos.zobrist_hash::<Zobrist64>(EnPassantMode::Legal).0
}

fn white_cp(s: Score) -> i32 {
    match s {
        Score::Cp(c) => c,
        Score::Mate(n) if n > 0 => 30_000,
        Score::Mate(n) if n < 0 => -30_000,
        Score::Mate(_) => 0,
    }
}

struct Engines {
    white: Engine,
    black: Engine,
    referee: Engine,
}

#[allow(clippy::too_many_arguments)]
fn pick_move(
    engine: &mut Engine,
    p: &Player,
    content: &Content,
    moves: &[String],
    pos: &Chess,
    history: &[Chess],
    rng: &mut StdRng,
    ref_nps: u64,
) -> Result<String, String> {
    match &p.kind {
        Kind::Bot { bot_id, elo } => choose_move_deterministic(engine, content, bot_id, START_FEN, moves, *elo, rng, ref_nps).map(|m| m.uci),
        Kind::Engine { nodes } => {
            let limits = SearchLimits { depth: None, movetime_ms: None, nodes: Some(*nodes), multipv: 1 };
            let stop = AtomicBool::new(false);
            let info = engine.search_with_history(pos, history, &limits, &stop, &mut |_| {});
            info.best_move().map(str::to_string).ok_or_else(|| "engine returned no move".to_string())
        }
        Kind::Random => {
            let legal = pos.legal_moves();
            if legal.is_empty() {
                return Err("no legal moves".into());
            }
            Ok(gm_engine::move_to_uci(&legal[rng.gen_range(0..legal.len())]))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn play(job: &Job, players: &[Player], content: &Content, opening: &[String], eng: &mut Engines, ref_nps: u64, max_plies: usize) -> Game {
    let (pw, pb) = (&players[job.white], &players[job.black]);
    eng.white.new_game();
    eng.black.new_game();
    eng.referee.new_game();
    let mut rng = StdRng::seed_from_u64(job.seed);
    let mut pos = Chess::default();
    let mut moves: Vec<String> = Vec::new();
    let mut history: Vec<Chess> = Vec::new();
    let mut reps: Vec<u64> = vec![zobrist(&pos)];
    let finish = |score: f64, plies: usize, reason: &str| Game {
        white: job.white,
        black: job.black,
        score,
        pair: job.pair,
        plies,
        reason: reason.into(),
    };
    for u in opening {
        let Ok(m) = uci_to_move(&pos, u) else { break };
        history.push(pos.clone());
        pos.play_unchecked(&m);
        moves.push(u.clone());
        reps.push(zobrist(&pos));
    }
    let mut resign_run: (i32, usize) = (0, 0);
    let mut draw_run = 0usize;
    let stop = AtomicBool::new(false);
    let referee_limits = SearchLimits { depth: None, movetime_ms: None, nodes: Some(REFEREE_NODES), multipv: 1 };
    loop {
        let plies = moves.len();
        if pos.is_checkmate() {
            return finish(if pos.turn() == Color::White { 0.0 } else { 1.0 }, plies, "mate");
        }
        if pos.is_stalemate() {
            return finish(0.5, plies, "stalemate");
        }
        if pos.is_insufficient_material() {
            return finish(0.5, plies, "material");
        }
        if pos.halfmoves() >= 100 {
            return finish(0.5, plies, "fifty");
        }
        let current = zobrist(&pos);
        if reps.iter().filter(|&&h| h == current).count() >= 3 {
            return finish(0.5, plies, "repetition");
        }
        // Referee eval (white POV) for adjudication.
        let cp = white_cp(eng.referee.search_with_history(&pos, &history, &referee_limits, &stop, &mut |_| {}).score());
        if plies >= max_plies {
            let s = if cp >= 300 { 1.0 } else if cp <= -300 { 0.0 } else { 0.5 };
            return finish(s, plies, "max-plies");
        }
        let sign = if cp >= RESIGN_CP { 1 } else if cp <= -RESIGN_CP { -1 } else { 0 };
        resign_run = if sign != 0 && sign == resign_run.0 { (sign, resign_run.1 + 1) } else { (sign, usize::from(sign != 0)) };
        if resign_run.1 >= RESIGN_PLIES {
            let winner = if resign_run.0 > 0 { pw } else { pb };
            if winner.nominal >= CONVERTS_FROM {
                return finish(if resign_run.0 > 0 { 1.0 } else { 0.0 }, plies, "resign");
            }
        }
        draw_run = if cp.abs() <= DRAW_CP { draw_run + 1 } else { 0 };
        if plies >= DRAW_FROM_PLY && draw_run >= DRAW_PLIES && pw.nominal.min(pb.nominal) >= DRAW_FROM_NOMINAL {
            return finish(0.5, plies, "draw-adj");
        }
        let (p, engine) = if pos.turn() == Color::White { (pw, &mut eng.white) } else { (pb, &mut eng.black) };
        let uci = match pick_move(engine, p, content, &moves, &pos, &history, &mut rng, ref_nps) {
            Ok(u) => u,
            // A player that cannot move in a live position forfeits (should never happen).
            Err(_) => return finish(if pos.turn() == Color::White { 0.0 } else { 1.0 }, plies, "error"),
        };
        let Ok(m) = uci_to_move(&pos, &uci) else {
            return finish(if pos.turn() == Color::White { 0.0 } else { 1.0 }, plies, "illegal");
        };
        history.push(pos.clone());
        let irreversible = m.is_zeroing();
        pos.play_unchecked(&m);
        moves.push(uci);
        if irreversible {
            reps.clear();
            history.clear();
        }
        reps.push(zobrist(&pos));
    }
}

fn run_games(players: &[Player], content: &Content, o: &Opts) -> Result<Vec<Game>, String> {
    let lines = openings(content);
    if lines.is_empty() {
        return Err(format!("no openings found under {}", o.data.display()));
    }
    let mut jobs = schedule(players, o, lines.len());
    // Expensive games first so the tail of the run stays short.
    jobs.sort_by(|a, b| b.cost.total_cmp(&a.cost).then(a.pair.cmp(&b.pair)));
    let total = jobs.len();
    let total_cost: f64 = jobs.iter().map(|j| j.cost).sum();
    eprintln!(
        "calibrate: {} players, {} games, {} openings, {} threads, ref {} nps",
        players.len(),
        total,
        lines.len(),
        o.threads,
        o.ref_nps
    );
    let next = AtomicUsize::new(0);
    let done = Mutex::new((0usize, 0.0f64));
    let results: Mutex<Vec<Option<Game>>> = Mutex::new(vec![None; total]);
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..o.threads.min(total.max(1)) {
            scope.spawn(|| {
                let mut eng = Engines { white: Engine::new(BOT_TT_MB), black: Engine::new(BOT_TT_MB), referee: Engine::new(REFEREE_TT_MB) };
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(k) else { break };
                    let g = play(job, players, content, &lines[job.opening], &mut eng, o.ref_nps, o.max_plies);
                    if let Ok(mut r) = results.lock() {
                        r[k] = Some(g);
                    }
                    if let Ok(mut d) = done.lock() {
                        d.0 += 1;
                        d.1 += job.cost;
                        if d.0 % 50 == 0 || d.0 == total {
                            let secs = started.elapsed().as_secs_f64();
                            let frac = (d.1 / total_cost).max(1e-9);
                            eprintln!(
                                "  {}/{} games  {:.0}s elapsed  ~{:.0}s left",
                                d.0,
                                total,
                                secs,
                                secs / frac - secs
                            );
                        }
                    }
                }
            });
        }
    });
    let results = results.into_inner().map_err(|_| "worker panicked".to_string())?;
    let mut games: Vec<Game> = results.into_iter().flatten().collect();
    games.sort_by(|a, b| a.pair.cmp(&b.pair).then(b.white.cmp(&a.white)));
    eprintln!("calibrate: played {} games in {:.0}s", games.len(), started.elapsed().as_secs_f64());
    Ok(games)
}

/// Days since 1970-01-01 -> (year, month, day) (Howard Hinnant's civil_from_days).
fn today() -> String {
    let days = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

fn summarize(raw: &RawGames, o: &Opts, wall_secs: Option<f64>, threads: usize) -> serde_json::Value {
    let players = &raw.players;
    let n = players.len();
    let results: Vec<GameResult> =
        raw.games.iter().map(|g| GameResult { white: g.white, black: g.black, score: g.score, pair: g.pair }).collect();
    let anchors: Vec<(usize, f64)> =
        players.iter().enumerate().filter(|(_, p)| p.ladder).filter_map(|(i, p)| p.label.map(|l| (i, f64::from(l)))).collect();
    let (ratings, ci) = fit_with_ci(n, &results, &anchors, o.bootstrap, 0.95, mix(raw.seed, 77));
    let mut games_of = vec![0usize; n];
    let mut points = vec![0.0f64; n];
    let mut pairs: HashMap<(usize, usize), (usize, f64)> = HashMap::new();
    let mut reasons: HashMap<String, usize> = HashMap::new();
    let mut plies = 0usize;
    for g in &raw.games {
        games_of[g.white] += 1;
        games_of[g.black] += 1;
        points[g.white] += g.score;
        points[g.black] += 1.0 - g.score;
        let (a, b, s) = if g.white < g.black { (g.white, g.black, g.score) } else { (g.black, g.white, 1.0 - g.score) };
        let e = pairs.entry((a, b)).or_insert((0, 0.0));
        e.0 += 1;
        e.1 += s;
        *reasons.entry(g.reason.clone()).or_insert(0) += 1;
        plies += g.plies;
    }
    let mut pair_list: Vec<_> = pairs.into_iter().collect();
    pair_list.sort_by_key(|((a, b), _)| (*a, *b));
    let round = |x: f64| (x * 10.0).round() / 10.0;
    println!("\n{:<16} {:>6} {:>8} {:>17} {:>6} {:>6}", "player", "label", "rating", "95% CI", "games", "score");
    let mut rows = Vec::new();
    for (i, p) in players.iter().enumerate() {
        let label = p.label.map(|l| l.to_string()).unwrap_or_else(|| "-".into());
        let score = if games_of[i] > 0 { 100.0 * points[i] / games_of[i] as f64 } else { 0.0 };
        println!(
            "{:<16} {:>6} {:>8.0} {:>8.0}..{:<7.0} {:>6} {:>5.1}%",
            p.id, label, ratings[i], ci[i].0, ci[i].1, games_of[i], score
        );
        rows.push(json!({
            "id": p.id,
            "kind": p.kind,
            "label": p.label,
            "ladder": p.ladder,
            "rating": ratings[i].round(),
            "ci95": [ci[i].0.round(), ci[i].1.round()],
            "games": games_of[i],
            "score_pct": round(score),
        }));
    }
    json!({
        "_comment": "Generated by gm-calibrate (see docs/BOT_CALIBRATION.md). Ratings are Elo maximum likelihood, anchored so the ladder bots' mean rating equals the mean of their labels; ci95 is a bootstrap over game pairs.",
        "date": today(),
        "seed": raw.seed,
        "ref_nps": raw.ref_nps,
        "max_plies": raw.max_plies,
        "threads": threads,
        "wall_seconds": wall_secs.map(|s| s.round()),
        "games": raw.games.len(),
        "avg_plies": if raw.games.is_empty() { 0.0 } else { round(plies as f64 / raw.games.len() as f64) },
        "end_reasons": reasons,
        "players": rows,
        "pairs": pair_list.iter().map(|((a, b), (g, s))| json!({
            "a": players[*a].id, "b": players[*b].id, "games": g, "score_a_pct": round(100.0 * s / *g as f64),
        })).collect::<Vec<_>>(),
    })
}

fn main() {
    let o = match parse_opts() {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("gm-calibrate: {e}");
            }
            eprintln!("usage: gm-calibrate [--games N] [--neighbors K] [--threads T] [--seed S] [--ref-nps N] [--max-plies N] [--sparky L] [--anchors L] [--only L] [--no-coaches] [--no-random] [--out F] [--games-out F] [--fit F] [--bootstrap N] [--data DIR]");
            std::process::exit(2);
        }
    };
    let (raw, wall) = if let Some(path) = &o.fit {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("gm-calibrate: {}: {e}", path.display());
                std::process::exit(1);
            }
        };
        match serde_json::from_str::<RawGames>(&text) {
            Ok(r) if r.games.iter().all(|g| g.white < r.players.len() && g.black < r.players.len()) => (r, None),
            Ok(_) => {
                eprintln!("gm-calibrate: {}: game refers to an unknown player", path.display());
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("gm-calibrate: {}: {e}", path.display());
                std::process::exit(1);
            }
        }
    } else {
        let content = match Content::load(&o.data) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("gm-calibrate: loading {}: {e}", o.data.display());
                std::process::exit(1);
            }
        };
        let players = roster(&o);
        let started = Instant::now();
        let games = match run_games(&players, &content, &o) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("gm-calibrate: {e}");
                std::process::exit(1);
            }
        };
        let raw = RawGames { players, games, seed: o.seed, ref_nps: o.ref_nps, max_plies: o.max_plies };
        if let Err(e) = write_json(&o.games_out, &raw) {
            eprintln!("gm-calibrate: {e}");
        }
        (raw, Some(started.elapsed().as_secs_f64()))
    };
    let summary = summarize(&raw, &o, wall, o.threads);
    match write_json(&o.out, &summary) {
        Ok(()) => eprintln!("calibrate: wrote {}", o.out.display()),
        Err(e) => {
            eprintln!("gm-calibrate: {e}");
            std::process::exit(1);
        }
    }
}
