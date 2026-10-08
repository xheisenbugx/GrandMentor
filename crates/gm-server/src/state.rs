//! Shared application state handed to every handler.

use std::sync::Arc;

use gm_analysis::GameReview;
use gm_content::Content;
use gm_engine::EnginePool;
use gm_mentor::Mentor;
use gm_store::Store;

use crate::cache::{BoundedCache, RecentSet};
use crate::puzzles::PuzzleIndex;

/// Max cached ad-hoc reviews (keyed by start fen + moves + depth).
const REVIEW_CACHE: usize = 16;
/// Max cached `/api/mentor/position` results.
const POSITION_CACHE: usize = 512;
/// How many recently served/attempted puzzles to avoid repeating.
const RECENT_PUZZLES: usize = 300;
/// Max cached opening lookups.
const OPENING_CACHE: usize = 2048;

/// Cached `/api/mentor/position` answer.
#[derive(Clone, Debug)]
pub struct PositionInsight {
    pub ideas: Vec<String>,
    pub eval: gm_engine::Score,
    pub best_line_san: Vec<String>,
}

/// Cheap to clone: every field is `Arc`-backed.
#[derive(Clone)]
pub struct AppState {
    pub content: Arc<Content>,
    pub pool: EnginePool,
    pub store: Store,
    pub mentor: Arc<Mentor>,
    pub puzzles: Arc<PuzzleIndex>,
    pub recent_puzzles: Arc<RecentSet<String>>,
    pub review_cache: Arc<BoundedCache<String, Arc<GameReview>>>,
    pub position_cache: Arc<BoundedCache<String, PositionInsight>>,
    pub opening_cache: Arc<BoundedCache<String, Option<gm_content::OpeningMatch>>>,
    /// Flipped to `true` on Ctrl-C so long-lived websockets close and shutdown completes.
    pub shutdown: Arc<tokio::sync::watch::Sender<bool>>,
}

impl AppState {
    pub fn new(content: Arc<Content>, pool: EnginePool, store: Store, mentor: Arc<Mentor>) -> Self {
        let puzzles = Arc::new(PuzzleIndex::build(&content));
        AppState {
            content,
            pool,
            store,
            mentor,
            puzzles,
            recent_puzzles: Arc::new(RecentSet::new(RECENT_PUZZLES)),
            review_cache: Arc::new(BoundedCache::new(REVIEW_CACHE)),
            position_cache: Arc::new(BoundedCache::new(POSITION_CACHE)),
            opening_cache: Arc::new(BoundedCache::new(OPENING_CACHE)),
            shutdown: Arc::new(tokio::sync::watch::Sender::new(false)),
        }
    }
}
