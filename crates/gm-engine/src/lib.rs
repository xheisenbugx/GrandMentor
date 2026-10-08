//! gm-engine: chess search + evaluation + a pool of engines for the server.
//!
//! The public API is fixed by `docs/CONTRACT.md` §3. Scores at the API boundary are always
//! from WHITE's point of view; `evaluate` is side-to-move POV.
//!
//! Modules: `search` (iterative deepening PVS), `eval` (tapered PeSTO + positional terms),
//! `tt` (fixed-size transposition table), `moves` (hashing, SEE, ordering helpers).

mod eval;
mod moves;
mod search;
mod tt;

use std::fmt;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use shakmaty::fen::Fen;
use shakmaty::san::SanPlus;
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Color, EnPassantMode, Move, Position};
use tokio::sync::Semaphore;

pub use shakmaty;

// ---------------------------------------------------------------------------------------------
// Score
// ---------------------------------------------------------------------------------------------

/// Engine score. On the wire: `{"cp": 35}` or `{"mate": 3}`.
/// `Mate(n)`: n > 0 = white mates in n moves, n < 0 = black mates in n moves.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Score {
    Cp(i32),
    Mate(i32),
}

impl Default for Score {
    fn default() -> Self {
        Score::Cp(0)
    }
}

impl Score {
    /// Centipawn value used for mate scores when a single number is needed.
    pub const MATE_CP: i32 = 100_000;

    /// Flip the point of view (white <-> black).
    pub fn negate(self) -> Score {
        match self {
            Score::Cp(c) => Score::Cp(-c),
            Score::Mate(m) => Score::Mate(-m),
        }
    }

    /// Convert a score given from `side`'s point of view into white's point of view.
    pub fn to_white_pov(self, side: Color) -> Score {
        match side {
            Color::White => self,
            Color::Black => self.negate(),
        }
    }

    /// Convert a white-POV score into `side`'s point of view.
    pub fn for_side(self, side: Color) -> Score {
        self.to_white_pov(side)
    }

    /// Single comparable number (white POV if the score is white POV). Faster mates rank higher.
    pub fn to_cp(self) -> i32 {
        match self {
            Score::Cp(c) => c,
            Score::Mate(m) if m > 0 => Self::MATE_CP - m.min(1000),
            Score::Mate(m) if m < 0 => -Self::MATE_CP - m.max(-1000),
            // Mate(0): side to move is already mated; treat as a loss for the side to move is
            // unknowable here, so map to a neutral 0.
            Score::Mate(_) => 0,
        }
    }

    /// Centipawns clamped to +-`limit` (mates map to the limit).
    pub fn to_cp_clamped(self, limit: i32) -> i32 {
        self.to_cp().clamp(-limit, limit)
    }

    pub fn is_mate(self) -> bool {
        matches!(self, Score::Mate(_))
    }
}

impl fmt::Display for Score {
    /// "+0.35", "-1.20", "M3", "-M2"
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Score::Cp(c) => write!(f, "{:+.2}", c as f64 / 100.0),
            Score::Mate(m) if m < 0 => write!(f, "-M{}", -m),
            Score::Mate(m) => write!(f, "M{m}"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Search types
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct SearchLimits {
    pub depth: Option<u8>,
    pub movetime_ms: Option<u64>,
    pub nodes: Option<u64>,
    pub multipv: usize,
}

impl Default for SearchLimits {
    fn default() -> Self {
        SearchLimits {
            depth: None,
            movetime_ms: None,
            nodes: None,
            multipv: 1,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct PvLine {
    pub score: Score,
    /// UCI moves
    pub moves: Vec<String>,
    /// SAN moves (same length as `moves`)
    pub san: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SearchInfo {
    pub depth: u8,
    pub seldepth: u8,
    pub nodes: u64,
    pub nps: u64,
    pub time_ms: u64,
    /// sorted best-first
    pub lines: Vec<PvLine>,
}

impl SearchInfo {
    /// Best line, if any.
    pub fn best(&self) -> Option<&PvLine> {
        self.lines.first()
    }
    /// Best move in UCI, if any.
    pub fn best_move(&self) -> Option<&str> {
        self.lines
            .first()
            .and_then(|l| l.moves.first())
            .map(String::as_str)
    }
    /// Score of the best line (white POV), `Cp(0)` if no lines (e.g. game over handled by caller).
    pub fn score(&self) -> Score {
        self.lines.first().map(|l| l.score).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------------------------
// Engine + evaluation (see search.rs / eval.rs)
// ---------------------------------------------------------------------------------------------

pub use search::{Engine, MAX_PLY};

/// Static evaluation in centipawns from the side-to-move's point of view.
pub fn evaluate(pos: &Chess) -> i32 {
    eval::evaluate(pos)
}

/// Static evaluation as a white-POV `Score::Cp`.
pub fn evaluate_white(pos: &Chess) -> Score {
    Score::Cp(eval::evaluate(pos)).to_white_pov(pos.turn())
}

/// 64-bit Zobrist key of a position (stable; matches shakmaty's Zobrist64 with legal ep).
pub fn position_hash(pos: &Chess) -> u64 {
    moves::full_hash(pos)
}

// ---------------------------------------------------------------------------------------------
// Notation helpers
// ---------------------------------------------------------------------------------------------

/// The standard starting position FEN.
pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Parse a FEN (also accepts `"start"` / `"startpos"` / empty for the initial position).
pub fn parse_fen(fen: &str) -> Result<Chess, String> {
    let fen = fen.trim();
    if fen.is_empty() || fen == "start" || fen == "startpos" {
        return Ok(Chess::default());
    }
    if fen.len() > 128 {
        return Err("FEN too long".into());
    }
    let parsed = Fen::from_ascii(fen.as_bytes()).map_err(|e| format!("invalid FEN: {e}"))?;
    parsed
        .into_position::<Chess>(CastlingMode::Standard)
        .map_err(|e| format!("illegal position: {e}"))
}

/// Full 6-field FEN. En passant square only when a legal en passant capture exists.
pub fn to_fen(pos: &Chess) -> String {
    Fen::from_position(pos.clone(), EnPassantMode::Legal).to_string()
}

/// First four FEN fields (placement, side, castling, ep) — a position key independent of clocks.
pub fn fen_key(fen: &str) -> String {
    fen.split_whitespace().take(4).collect::<Vec<_>>().join(" ")
}

/// Parse a UCI move (`e2e4`, `e7e8q`, castling as `e1g1` or `e1h1`) and check legality.
pub fn uci_to_move(pos: &Chess, uci: &str) -> Result<Move, String> {
    let uci = uci.trim();
    if uci.len() < 4 || uci.len() > 5 {
        return Err(format!("invalid UCI move: {uci:?}"));
    }
    let parsed =
        UciMove::from_ascii(uci.as_bytes()).map_err(|_| format!("invalid UCI move: {uci:?}"))?;
    parsed
        .to_move(pos)
        .map_err(|_| format!("illegal move: {uci}"))
}

/// Parse a SAN move (e.g. `Nf3`, `exd5`, `O-O`, `e8=Q+`) and check legality.
pub fn san_to_move(pos: &Chess, san: &str) -> Result<Move, String> {
    let san = san.trim();
    if san.is_empty() || san.len() > 12 {
        return Err(format!("invalid SAN move: {san:?}"));
    }
    let parsed =
        SanPlus::from_ascii(san.as_bytes()).map_err(|_| format!("invalid SAN move: {san:?}"))?;
    parsed
        .san
        .to_move(pos)
        .map_err(|_| format!("illegal move: {san}"))
}

/// SAN including check/mate suffix (`Nf3`, `Qxf7#`).
pub fn move_to_san(pos: &Chess, m: &Move) -> String {
    SanPlus::from_move(pos.clone(), m).to_string()
}

/// Standard UCI (castling as king-to-destination, e.g. `e1g1`).
pub fn move_to_uci(m: &Move) -> String {
    UciMove::from_move(m, CastlingMode::Standard).to_string()
}

/// Replay UCI moves from `start`, returning the final position and the SAN of each move.
pub fn replay_uci(start: &Chess, moves: &[String]) -> Result<(Chess, Vec<String>), String> {
    let mut pos = start.clone();
    let mut sans = Vec::with_capacity(moves.len());
    for (i, u) in moves.iter().enumerate() {
        let m = uci_to_move(&pos, u).map_err(|e| format!("ply {}: {e}", i + 1))?;
        sans.push(move_to_san(&pos, &m));
        pos.play_unchecked(&m);
    }
    Ok((pos, sans))
}

/// Convert a UCI line from `pos` into SAN, stopping at the first illegal move.
pub fn uci_line_to_san(pos: &Chess, moves: &[String]) -> Vec<String> {
    let mut p = pos.clone();
    let mut out = Vec::with_capacity(moves.len());
    for u in moves {
        match uci_to_move(&p, u) {
            Ok(m) => {
                out.push(move_to_san(&p, &m));
                p.play_unchecked(&m);
            }
            Err(_) => break,
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// EnginePool
// ---------------------------------------------------------------------------------------------

struct PoolInner {
    engines: Mutex<Vec<Engine>>,
    permits: Arc<Semaphore>,
    size: usize,
    tt_mb: usize,
}

/// A fixed set of engines. Cloning is cheap (shared `Arc`).
#[derive(Clone)]
pub struct EnginePool {
    inner: Arc<PoolInner>,
}

impl fmt::Debug for EnginePool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnginePool")
            .field("size", &self.inner.size)
            .field("tt_mb", &self.inner.tt_mb)
            .finish()
    }
}

/// Returns the engine to the pool on drop (also during a panic unwind).
struct Lease {
    engine: Option<Engine>,
    pool: Arc<PoolInner>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let engine = match self.engine.take() {
            // After a panic the engine may be in an inconsistent state: replace it.
            Some(_) if std::thread::panicking() => Engine::new(self.pool.tt_mb),
            Some(e) => e,
            None => Engine::new(self.pool.tt_mb),
        };
        self.pool.engines.lock().push(engine);
    }
}

impl EnginePool {
    pub fn new(size: usize, tt_mb: usize) -> Self {
        let size = size.max(1);
        let engines = (0..size).map(|_| Engine::new(tt_mb)).collect();
        EnginePool {
            inner: Arc::new(PoolInner {
                engines: Mutex::new(engines),
                permits: Arc::new(Semaphore::new(size)),
                size,
                tt_mb,
            }),
        }
    }

    /// Run blocking work with an exclusive engine on a blocking thread.
    /// If the closure panics, the panic is propagated to the caller and the engine is replaced.
    pub async fn with_engine<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Engine) -> R + Send + 'static,
    ) -> R {
        let inner = Arc::clone(&self.inner);
        let permit = match Arc::clone(&self.inner.permits).acquire_owned().await {
            Ok(p) => p,
            Err(_) => unreachable_closed(),
        };
        let handle = tokio::task::spawn_blocking(move || {
            let _permit = permit; // dropped after `lease` (reverse declaration order)
            let engine = inner
                .engines
                .lock()
                .pop()
                .unwrap_or_else(|| Engine::new(inner.tt_mb));
            let mut lease = Lease {
                engine: Some(engine),
                pool: Arc::clone(&inner),
            };
            let engine_ref = match lease.engine.as_mut() {
                Some(e) => e,
                None => unreachable_closed(),
            };
            f(engine_ref)
        });
        match handle.await {
            Ok(r) => r,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(e) => panic!("engine task cancelled (runtime shutting down): {e}"),
        }
    }

    pub fn size(&self) -> usize {
        self.inner.size
    }

    /// Number of engines currently idle.
    pub fn available(&self) -> usize {
        self.inner.permits.available_permits()
    }
}

#[cold]
fn unreachable_closed() -> ! {
    // The semaphore is never closed and the lease always holds an engine; this cannot happen.
    panic!("engine pool invariant violated")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn fen_roundtrip() {
        let p = parse_fen("start").unwrap();
        assert_eq!(to_fen(&p), START_FEN);
        let f = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
        assert_eq!(to_fen(&parse_fen(f).unwrap()), f);
        assert!(parse_fen("garbage").is_err());
        assert!(parse_fen("8/8/8/8/8/8/8/8 w - - 0 1").is_err());
    }

    #[test]
    fn uci_san() {
        let p = Chess::default();
        let m = uci_to_move(&p, "e2e4").unwrap();
        assert_eq!(move_to_san(&p, &m), "e4");
        assert_eq!(move_to_uci(&m), "e2e4");
        assert!(uci_to_move(&p, "e2e5").is_err());
        assert!(uci_to_move(&p, "zz").is_err());
        let castle = parse_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        let m = uci_to_move(&castle, "e1g1").unwrap();
        assert_eq!(move_to_uci(&m), "e1g1");
        assert_eq!(move_to_san(&castle, &m), "O-O");
        let m = san_to_move(&castle, "O-O-O").unwrap();
        assert_eq!(move_to_uci(&m), "e1c1");
    }

    #[test]
    fn score_json() {
        assert_eq!(
            serde_json::to_string(&Score::Cp(35)).unwrap(),
            r#"{"cp":35}"#
        );
        assert_eq!(
            serde_json::to_string(&Score::Mate(-2)).unwrap(),
            r#"{"mate":-2}"#
        );
        let s: Score = serde_json::from_str(r#"{"mate":3}"#).unwrap();
        assert_eq!(s, Score::Mate(3));
        assert!(Score::Mate(1).to_cp() > Score::Mate(3).to_cp());
        assert!(Score::Mate(-1).to_cp() < Score::Mate(-3).to_cp());
    }

    #[test]
    fn finds_mate_in_one() {
        // White: Qh5 with Bc4 -> Qxf7#
        let p =
            parse_fen("r1bqkbnr/pppp1ppp/2n5/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4").unwrap();
        let mut e = Engine::new(1);
        let stop = AtomicBool::new(false);
        let info = e.search(
            &p,
            &SearchLimits {
                depth: Some(2),
                ..Default::default()
            },
            &stop,
            &mut |_| {},
        );
        assert_eq!(info.best_move(), Some("h5f7"));
        assert_eq!(info.score(), Score::Mate(1));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pool_works() {
        let pool = EnginePool::new(2, 1);
        let mut handles = Vec::new();
        for _ in 0..6 {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                pool.with_engine(|e| {
                    let stop = AtomicBool::new(false);
                    e.search(
                        &Chess::default(),
                        &SearchLimits {
                            depth: Some(1),
                            ..Default::default()
                        },
                        &stop,
                        &mut |_| {},
                    )
                    .lines
                    .len()
                })
                .await
            }));
        }
        for h in handles {
            assert_eq!(h.await.unwrap(), 1);
        }
        assert_eq!(pool.available(), 2);
        assert_eq!(pool.inner.engines.lock().len(), 2);
    }
}
