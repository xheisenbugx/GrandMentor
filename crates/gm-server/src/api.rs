//! REST handlers for `/api/*` (CONTRACT §4).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shakmaty::Position;

use gm_analysis::GameReview;
use gm_content::{Lang, Opening, OpeningMatch, Puzzle};
use gm_engine::{fen_key, parse_fen, to_fen, EnginePool, Score, SearchInfo, SearchLimits};
use gm_store::{GamePatch, GameQuery, NewGame, ProfilePatch, Store};

use crate::error::{ApiError, ApiJson, ApiPath, ApiQuery, ApiResult};
use crate::lang::ReqLang;
use crate::puzzles::{today_utc, ThemeCount};
use crate::state::{AppState, PositionInsight};

// ---------------------------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------------------------

/// Max plies accepted for games / bot move requests / reviews.
pub const MAX_PLIES: usize = 1200;
/// Max plies analysed by a review (keeps a review bounded in time).
pub const MAX_REVIEW_PLIES: usize = 600;
/// `/api/engine/analyze` movetime cap (ms).
pub const HTTP_MAX_MOVETIME_MS: u64 = 10_000;
pub const MAX_MULTIPV: usize = 5;
pub const DEFAULT_REVIEW_DEPTH: u8 = 14;
pub const MAX_REVIEW_DEPTH: u8 = 22;

// ---------------------------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/bots", get(bots))
        .route("/bot/move", post(bot_move))
        .route("/engine/analyze", post(engine_analyze))
        .route("/engine/ws", get(crate::ws::engine_ws))
        .route("/review", post(review))
        .route("/mentor/chat", post(mentor_chat))
        .route("/mentor/explain", post(mentor_explain))
        .route("/mentor/position", get(mentor_position))
        .route("/games", get(list_games).post(create_game))
        .route("/games/import", post(import_games))
        .route(
            "/games/:id",
            get(get_game).put(update_game).delete(delete_game),
        )
        .route("/games/:id/pgn", get(game_pgn))
        .route("/puzzles/next", get(puzzle_next))
        .route("/puzzles/daily", get(puzzle_daily))
        .route("/puzzles/themes", get(puzzle_themes))
        .route("/puzzles/rush", get(puzzle_rush).post(record_rush))
        .route("/puzzles/:id", get(get_puzzle))
        .route("/puzzles/:id/attempt", post(puzzle_attempt))
        .route("/courses", get(courses))
        .route("/courses/:id", get(course))
        .route("/progress", get(get_progress).post(set_progress))
        .route("/openings", get(openings))
        .route("/openings/lookup", get(opening_lookup))
        .route("/openings/:id", get(opening))
        .route("/endgames", get(endgames))
        .route("/endgames/:id", get(endgame))
        .route("/profile", get(get_profile).put(update_profile))
        .route("/stats", get(stats))
        .merge(crate::routes::router())
        .fallback(api_not_found)
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("no such API endpoint")
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// Run blocking work (SQLite, content scans) off the async workers. A panic inside becomes a 500.
pub async fn blocking<R, F>(f: F) -> ApiResult<R>
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        if e.is_panic() {
            ApiError::internal("internal error (worker panicked)")
        } else {
            ApiError::unavailable("server is shutting down")
        }
    })
}

/// Run a store operation on the blocking pool, mapping errors to `{error}`.
pub(crate) async fn store_op<R, F>(store: &Store, f: F) -> ApiResult<R>
where
    R: Send + 'static,
    F: FnOnce(&Store) -> anyhow::Result<R> + Send + 'static,
{
    let store = store.clone();
    blocking(move || f(&store)).await?.map_err(ApiError::from)
}

/// Sets the stop flag when dropped — if the HTTP client disconnects, the handler future is
/// dropped and the running search is cancelled instead of burning an engine.
pub struct StopOnDrop(pub Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Run work with a pooled engine. Panics inside the engine become a 500 (the pool replaces the
/// engine); always bounded by the pool size.
pub async fn with_engine<R, F>(pool: &EnginePool, f: F) -> ApiResult<R>
where
    R: Send + 'static,
    F: FnOnce(&mut gm_engine::Engine) -> R + Send + 'static,
{
    let pool = pool.clone();
    tokio::spawn(async move { pool.with_engine(f).await })
        .await
        .map_err(|e| {
            if e.is_panic() {
                ApiError::internal("engine error")
            } else {
                ApiError::unavailable("server is shutting down")
            }
        })
}

pub(crate) fn parse_position(fen: &str) -> ApiResult<shakmaty::Chess> {
    parse_fen(fen).map_err(ApiError::bad_request)
}

pub(crate) fn validate_moves(start_fen: &str, moves: &[String], max: usize) -> ApiResult<shakmaty::Chess> {
    if moves.len() > max {
        return Err(ApiError::bad_request(format!("too many moves (max {max})")));
    }
    let start = parse_position(start_fen)?;
    let (pos, _) = gm_engine::replay_uci(&start, moves).map_err(ApiError::bad_request)?;
    Ok(pos)
}

/// Truncate to at most `max` chars (on a char boundary).
pub(crate) fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

fn opt_trim(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn parse_opt<T: std::str::FromStr>(name: &str, s: &Option<String>) -> ApiResult<Option<T>> {
    match opt_trim(s) {
        None => Ok(None),
        Some(v) => v
            .parse::<T>()
            .map(Some)
            .map_err(|_| ApiError::bad_request(format!("invalid `{name}`: {v:?}"))),
    }
}

fn parse_bool(name: &str, s: &Option<String>) -> ApiResult<Option<bool>> {
    match opt_trim(s).map(str::to_ascii_lowercase).as_deref() {
        None => Ok(None),
        Some("true" | "1" | "yes" | "on") => Ok(Some(true)),
        Some("false" | "0" | "no" | "off") => Ok(Some(false)),
        Some(v) => Err(ApiError::bad_request(format!("invalid `{name}`: {v:?}"))),
    }
}

const VALID_RESULTS: [&str; 4] = ["1-0", "0-1", "1/2-1/2", "*"];

// ---------------------------------------------------------------------------------------------
// Health & bots
// ---------------------------------------------------------------------------------------------

async fn health(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "llm_enabled": st.mentor.llm_enabled(),
        "engines": st.pool.size(),
        "engines_idle": st.pool.available(),
        "content": {
            "openings": st.content.openings.len(),
            "puzzles": st.content.puzzles.len(),
            "courses": st.content.courses.len(),
            "endgames": st.content.endgames.len(),
        },
    }))
}

async fn bots(State(st): State<AppState>, ReqLang(lang): ReqLang) -> Json<Vec<gm_bots::BotProfile>> {
    let mut bots = gm_bots::list(lang);
    // The adaptive bot's rating is its current, per-user level.
    if let Some(b) = bots.iter_mut().find(|b| b.id == gm_bots::ADAPTIVE_ID) {
        b.elo = crate::routes::adaptive::adaptive_level(&st).await;
    }
    Json(bots)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BotMoveReq {
    bot_id: String,
    start_fen: String,
    moves: Vec<String>,
}

async fn bot_move(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(req): ApiJson<BotMoveReq>,
) -> ApiResult<Json<gm_bots::BotMove>> {
    if !gm_bots::exists(&req.bot_id) {
        return Err(ApiError::not_found(format!(
            "unknown bot: {:?}",
            req.bot_id
        )));
    }
    let pos = validate_moves(&req.start_fen, &req.moves, MAX_PLIES)?;
    if pos.is_game_over() {
        return Err(ApiError::bad_request("the game is already over"));
    }
    let content = Arc::clone(&st.content);
    // The adaptive bot plays at its stored level (see routes/adaptive.rs).
    let elo = if req.bot_id == gm_bots::ADAPTIVE_ID {
        Some(crate::routes::adaptive::adaptive_level(&st).await)
    } else {
        None
    };
    let mv = with_engine(&st.pool, move |engine| {
        gm_bots::choose_move_at(engine, &content, &req.bot_id, &req.start_fen, &req.moves, elo, lang)
    })
    .await?
    .map_err(ApiError::bad_request)?;
    Ok(Json(mv))
}

// ---------------------------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
struct AnalyzeReq {
    fen: String,
    depth: Option<u8>,
    movetime_ms: Option<u64>,
    nodes: Option<u64>,
    multipv: Option<usize>,
}

/// Normalise client limits: multipv 1..=5, depth 1..=60, movetime capped (and always set so a
/// search can never run unbounded).
pub fn clamp_limits(
    depth: Option<u8>,
    movetime_ms: Option<u64>,
    nodes: Option<u64>,
    multipv: Option<usize>,
    max_ms: u64,
    default_ms: u64,
) -> SearchLimits {
    let depth = depth.map(|d| d.clamp(1, 60));
    let movetime = match (depth, movetime_ms, nodes) {
        (_, Some(ms), _) => ms.clamp(10, max_ms),
        (None, None, None) => default_ms.min(max_ms),
        _ => max_ms, // depth/nodes-limited: still hard-capped in time
    };
    SearchLimits {
        depth,
        movetime_ms: Some(movetime),
        nodes: nodes.map(|n| n.clamp(1, 2_000_000_000)),
        multipv: multipv.unwrap_or(1).clamp(1, MAX_MULTIPV),
    }
}

async fn engine_analyze(
    State(st): State<AppState>,
    ApiJson(req): ApiJson<AnalyzeReq>,
) -> ApiResult<Json<SearchInfo>> {
    let pos = parse_position(&req.fen)?;
    let limits = clamp_limits(
        req.depth,
        req.movetime_ms,
        req.nodes,
        req.multipv,
        HTTP_MAX_MOVETIME_MS,
        1_000,
    );
    let stop = Arc::new(AtomicBool::new(false));
    let _guard = StopOnDrop(Arc::clone(&stop));
    let info = with_engine(&st.pool, move |engine| {
        engine.search(&pos, &limits, &stop, &mut |_| {})
    })
    .await?;
    Ok(Json(info))
}

// ---------------------------------------------------------------------------------------------
// Review
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
struct ReviewReq {
    start_fen: Option<String>,
    moves: Option<Vec<String>>,
    pgn: Option<String>,
    game_id: Option<i64>,
    depth: Option<u8>,
    /// Recompute even when the game already has a cached review.
    force: bool,
}

pub(crate) async fn run_review(
    st: &AppState,
    start_fen: String,
    moves: Vec<String>,
    depth: u8,
    lang: Lang,
) -> ApiResult<GameReview> {
    let pool = st.pool.clone();
    let content = Arc::clone(&st.content);
    // One stop flag for the whole review. It is raised when this handler future is dropped
    // (client disconnected / navigated away) and when the server starts shutting down, so a
    // review never keeps burning engines for nobody.
    let stop = Arc::new(AtomicBool::new(false));
    let _guard = StopOnDrop(Arc::clone(&stop));
    let mut shutdown = st.shutdown.subscribe();
    let flag = Arc::clone(&stop);
    // Spawned so a panic in the analysis crate becomes a 500 rather than a dropped connection.
    let handle = tokio::spawn(async move {
        let review =
            gm_analysis::review_game_cancellable(&pool, content, &start_fen, &moves, depth, None, lang, flag);
        tokio::select! {
            r = review => r,
            // Dropping the review future raises its stop flag too.
            _ = shutdown.wait_for(|v| *v) => Err(gm_analysis::REVIEW_CANCELLED.to_string()),
        }
    });
    let res = handle.await.map_err(|e| {
        if e.is_panic() {
            ApiError::internal("analysis error")
        } else {
            ApiError::unavailable("server is shutting down")
        }
    })?;
    match res {
        Ok(review) => Ok(review),
        Err(e) if e == gm_analysis::REVIEW_CANCELLED => Err(ApiError::unavailable("the review was cancelled")),
        Err(e) => Err(ApiError::bad_request(e)),
    }
}

/// Re-localize a review's text (explanations, summary, opening names) without the engine.
async fn relocalized(st: &AppState, review: GameReview, lang: Lang) -> ApiResult<GameReview> {
    let content = Arc::clone(&st.content);
    blocking(move || gm_analysis::relocalize(&review, &content, lang)).await
}

async fn review(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(req): ApiJson<ReviewReq>,
) -> ApiResult<Json<GameReview>> {
    let depth = req
        .depth
        .unwrap_or(DEFAULT_REVIEW_DEPTH)
        .clamp(1, MAX_REVIEW_DEPTH);

    if let Some(id) = req.game_id {
        let game = store_op(&st.store, move |s| s.get_game(id))
            .await?
            .ok_or_else(|| ApiError::not_found(format!("game {id} not found")))?;
        if !req.force {
            if let Some((cached, cached_lang)) = game
                .review_json
                .as_deref()
                .and_then(gm_analysis::from_stored_json)
            {
                if cached.moves.len() == game.moves.len().min(MAX_REVIEW_PLIES) {
                    if cached_lang == lang {
                        return Ok(Json(cached));
                    }
                    // Same game, other language: rewrite only the text, never re-run the engine.
                    let review = relocalized(&st, cached, lang).await?;
                    let patch = GamePatch {
                        review_json: gm_analysis::to_stored_json(&review, lang),
                        ..Default::default()
                    };
                    if let Err(e) = store_op(&st.store, move |s| s.update_game(id, &patch)).await {
                        tracing::warn!("could not cache relocalized review for game {id}: {}", e.message);
                    }
                    return Ok(Json(review));
                }
            }
        }
        let start_fen = if game.start_fen.trim().is_empty() {
            gm_engine::START_FEN.to_string()
        } else {
            game.start_fen.clone()
        };
        let mut moves = game.moves.clone();
        moves.truncate(MAX_REVIEW_PLIES);
        validate_moves(&start_fen, &moves, MAX_REVIEW_PLIES)?;
        let review = run_review(&st, start_fen, moves, depth, lang).await?;
        let patch = GamePatch {
            review_json: gm_analysis::to_stored_json(&review, lang),
            accuracy_white: Some(review.white.accuracy),
            accuracy_black: Some(review.black.accuracy),
            ..Default::default()
        };
        // Caching is best-effort: the review itself is still returned on a store error.
        if let Err(e) = store_op(&st.store, move |s| s.update_game(id, &patch)).await {
            tracing::warn!("could not cache review for game {id}: {}", e.message);
        } else {
            // "Learn from your mistakes": the user's errors become spaced-repetition cards.
            crate::routes::mistakes::ingest_review(&st, &game, &review, lang).await;
        }
        return Ok(Json(review));
    }

    let (start_fen, moves) = if let Some(pgn) = opt_trim(&req.pgn).map(str::to_string) {
        let games = blocking(move || gm_store::pgn::parse_pgn(&pgn))
            .await?
            .map_err(ApiError::bad_request)?;
        let g = games
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::bad_request("no game found in PGN"))?;
        (g.start_fen, g.moves)
    } else if let Some(moves) = req.moves {
        (req.start_fen.unwrap_or_default(), moves)
    } else {
        return Err(ApiError::bad_request("provide `moves`, `pgn` or `game_id`"));
    };
    let start_fen = if start_fen.trim().is_empty() {
        gm_engine::START_FEN.to_string()
    } else {
        start_fen
    };
    let start_pos = parse_position(&start_fen)?;
    let start_fen = to_fen(&start_pos);
    if moves.len() > MAX_REVIEW_PLIES {
        return Err(ApiError::bad_request(format!(
            "game too long to review (max {MAX_REVIEW_PLIES} plies)"
        )));
    }
    validate_moves(&start_fen, &moves, MAX_REVIEW_PLIES)?;

    let key = format!("{depth}|{start_fen}|{}", moves.join(","));
    if !req.force {
        if let Some(hit) = st.review_cache.get(&key) {
            let (cached_lang, cached) = &*hit;
            if *cached_lang == lang {
                return Ok(Json(cached.clone()));
            }
            let review = relocalized(&st, cached.clone(), lang).await?;
            st.review_cache.insert(key, Arc::new((lang, review.clone())));
            return Ok(Json(review));
        }
    }
    let review = run_review(&st, start_fen, moves, depth, lang).await?;
    st.review_cache.insert(key, Arc::new((lang, review.clone())));
    Ok(Json(review))
}

// ---------------------------------------------------------------------------------------------
// Mentor
// ---------------------------------------------------------------------------------------------

async fn mentor_chat(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(mut req): ApiJson<gm_mentor::ChatRequest>,
) -> ApiResult<Json<gm_mentor::ChatResponse>> {
    req.question = clip(req.question.trim(), 2_000);
    if req.question.is_empty() {
        return Err(ApiError::bad_request("question must not be empty"));
    }
    if !req.fen.trim().is_empty() {
        parse_position(&req.fen)?;
    }
    if req.moves_san.len() > MAX_PLIES {
        let skip = req.moves_san.len() - MAX_PLIES;
        req.moves_san.drain(..skip);
    }
    req.engine_lines.truncate(MAX_MULTIPV);
    for l in &mut req.engine_lines {
        *l = clip(l, 300);
    }
    if req.history.len() > 20 {
        let skip = req.history.len() - 20;
        req.history.drain(..skip);
    }
    for t in &mut req.history {
        t.text = clip(&t.text, 4_000);
    }
    let mentor = Arc::clone(&st.mentor);
    let resp = tokio::spawn(async move { mentor.chat(req, lang).await })
        .await
        .map_err(|_| ApiError::internal("mentor error"))?;
    Ok(Json(resp))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ExplainReq {
    fen: String,
    move_uci: String,
}

#[derive(Serialize)]
struct ExplainResp {
    classification: gm_analysis::Classification,
    explanation: String,
    played_san: String,
    best_move_uci: String,
    best_move_san: String,
    best_line_san: Vec<String>,
    eval_before: Score,
    eval_after: Score,
}

async fn mentor_explain(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(req): ApiJson<ExplainReq>,
) -> ApiResult<Json<ExplainResp>> {
    let pos = parse_position(&req.fen)?;
    let fen = to_fen(&pos);
    let uci = req.move_uci.trim().to_string();
    gm_engine::uci_to_move(&pos, &uci).map_err(ApiError::bad_request)?;
    let review = run_review(&st, fen, vec![uci], 12, lang).await?;
    let m = review
        .moves
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::internal("analysis returned no moves"))?;
    Ok(Json(ExplainResp {
        classification: m.classification,
        explanation: m.explanation,
        played_san: m.san,
        best_move_uci: m.best_move_uci,
        best_move_san: m.best_move_san,
        best_line_san: m.best_line_san,
        eval_before: m.eval_before,
        eval_after: m.eval_after,
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FenQuery {
    fen: Option<String>,
}

#[derive(Serialize)]
struct PositionResp {
    ideas: Vec<String>,
    eval: Score,
    best_line_san: Vec<String>,
}

async fn mentor_position(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<FenQuery>,
) -> ApiResult<Json<PositionResp>> {
    let fen = opt_trim(&q.fen).ok_or_else(|| ApiError::bad_request("missing `fen`"))?;
    let pos = parse_position(fen)?;
    let fen = to_fen(&pos);
    let key = fen_key(&fen);
    if let Some(hit) = st.position_cache.get(&key) {
        // The engine part is language-independent; ideas are cheap rule-based text.
        let ideas = if hit.lang == lang {
            hit.ideas
        } else {
            let f = fen.clone();
            blocking(move || gm_mentor::describe_position(&f, lang)).await?
        };
        return Ok(Json(PositionResp {
            ideas,
            eval: hit.eval,
            best_line_san: hit.best_line_san,
        }));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let _guard = StopOnDrop(Arc::clone(&stop));
    let fen2 = fen.clone();
    let insight = with_engine(&st.pool, move |engine| {
        let ideas = gm_mentor::describe_position(&fen2, lang);
        let limits = SearchLimits {
            depth: Some(16),
            movetime_ms: Some(800),
            nodes: None,
            multipv: 1,
        };
        let info = engine.search(&pos, &limits, &stop, &mut |_| {});
        let best = info
            .best()
            .map(|l| l.san.iter().take(8).cloned().collect())
            .unwrap_or_default();
        PositionInsight {
            lang,
            ideas,
            eval: info.score(),
            best_line_san: best,
        }
    })
    .await?;
    st.position_cache.insert(key, insight.clone());
    Ok(Json(PositionResp {
        ideas: insight.ideas,
        eval: insight.eval,
        best_line_san: insight.best_line_san,
    }))
}

// ---------------------------------------------------------------------------------------------
// Games
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
struct GamesQueryRaw {
    search: Option<String>,
    result: Option<String>,
    bot_id: Option<String>,
    favorite: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
}

async fn list_games(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<GamesQueryRaw>,
) -> ApiResult<Json<Vec<gm_store::GameSummary>>> {
    let query = GameQuery {
        search: opt_trim(&q.search).map(|s| clip(s, 200)),
        result: opt_trim(&q.result).map(str::to_string),
        bot_id: opt_trim(&q.bot_id).map(str::to_string),
        favorite: parse_bool("favorite", &q.favorite)?,
        limit: Some(
            parse_opt::<u32>("limit", &q.limit)?
                .unwrap_or(50)
                .clamp(1, 500),
        ),
        offset: parse_opt::<u32>("offset", &q.offset)?,
    };
    let mut games = store_op(&st.store, move |s| s.list_games(&query)).await?;
    for g in &mut games {
        localize_opening_name(&st, lang, &mut g.opening_name);
    }
    Ok(Json(games))
}

/// Opening names are stored in the language the game was saved in; show them in the reader's.
/// Unknown names (e.g. from an imported PGN's Opening tag) are returned unchanged.
fn localize_opening_name(st: &AppState, lang: Lang, name: &mut Option<String>) {
    let Some(stored) = name.as_deref() else { return };
    let id = Lang::ALL.iter().find_map(|&l| {
        st.content
            .localized(l)
            .openings
            .iter()
            .find(|o| o.name == stored)
            .map(|o| o.id.clone())
    });
    if let Some(o) = id.and_then(|id| st.content.localized(lang).opening(&id).map(|o| o.name.clone())) {
        *name = Some(o);
    }
}

/// Deepest named opening reached in the first 30 plies.
fn detect_opening(
    content: &gm_content::Content,
    start: &shakmaty::Chess,
    moves: &[String],
) -> Option<String> {
    let mut pos = start.clone();
    let mut fens = Vec::new();
    for u in moves.iter().take(30) {
        let m = gm_engine::uci_to_move(&pos, u).ok()?;
        pos.play_unchecked(&m);
        fens.push(to_fen(&pos));
    }
    fens.iter()
        .rev()
        .find_map(|f| content.lookup_opening(f))
        .map(|m| m.opening.name)
}

async fn create_game(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(mut g): ApiJson<NewGame>,
) -> ApiResult<(StatusCode, Json<gm_store::GameRecord>)> {
    if g.result.trim().is_empty() {
        g.result = "*".into();
    }
    if !VALID_RESULTS.contains(&g.result.trim()) {
        return Err(ApiError::bad_request(format!(
            "invalid result {:?} (use 1-0, 0-1, 1/2-1/2 or *)",
            g.result
        )));
    }
    g.result = g.result.trim().to_string();
    let start = parse_position(&g.start_fen)?;
    g.start_fen = to_fen(&start);
    validate_moves(&g.start_fen, &g.moves, MAX_PLIES)?;
    g.white = clip(g.white.trim(), 100);
    g.black = clip(g.black.trim(), 100);
    g.notes = clip(&g.notes, 20_000);
    g.tags.truncate(32);
    if g.opening_name
        .as_deref()
        .is_none_or(|s| s.trim().is_empty())
    {
        let content = st.content.localized(lang);
        let moves = g.moves.clone();
        g.opening_name = blocking(move || detect_opening(&content, &start, &moves)).await?;
    }
    let rec = store_op(&st.store, move |s| {
        let rec = s.create_game(&g)?;
        // Activity feeds the daily plan / streaks; never fail the request because of it.
        let kind = if g.bot_id.is_some() { "game" } else { "local_game" };
        let _ = s.log_activity(kind, 1);
        Ok(rec)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(rec)))
}

async fn get_game(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiPath(id): ApiPath<i64>,
) -> ApiResult<Json<gm_store::GameRecord>> {
    let mut g = store_op(&st.store, move |s| s.get_game(id))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("game {id} not found")))?;
    localize_opening_name(&st, lang, &mut g.opening_name);
    Ok(Json(g))
}

async fn update_game(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(patch): ApiJson<GamePatch>,
) -> ApiResult<Json<gm_store::GameRecord>> {
    if let Some(r) = &patch.result {
        if !VALID_RESULTS.contains(&r.as_str()) {
            return Err(ApiError::bad_request(format!("invalid result {r:?}")));
        }
    }
    if let Some(moves) = &patch.moves {
        let start = store_op(&st.store, move |s| s.get_game(id))
            .await?
            .ok_or_else(|| ApiError::not_found(format!("game {id} not found")))?
            .start_fen;
        validate_moves(&start, moves, MAX_PLIES)?;
    }
    if let Some(n) = &patch.notes {
        if n.chars().count() > 20_000 {
            return Err(ApiError::bad_request(
                "notes too long (max 20000 characters)",
            ));
        }
    }
    if patch.tags.as_ref().is_some_and(|t| t.len() > 32) {
        return Err(ApiError::bad_request("too many tags (max 32)"));
    }
    store_op(&st.store, move |s| s.update_game(id, &patch))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("game {id} not found")))
}

async fn delete_game(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> ApiResult<Json<Value>> {
    if store_op(&st.store, move |s| s.delete_game(id)).await? {
        Ok(Json(json!({ "deleted": true })))
    } else {
        Err(ApiError::not_found(format!("game {id} not found")))
    }
}

async fn game_pgn(State(st): State<AppState>, ApiPath(id): ApiPath<i64>) -> ApiResult<Response> {
    let game = store_op(&st.store, move |s| s.get_game(id))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("game {id} not found")))?;
    let pgn = if game.pgn.trim().is_empty() {
        let headers = vec![
            ("Event".to_string(), "GrandMentor game".to_string()),
            ("White".to_string(), game.white.clone()),
            ("Black".to_string(), game.black.clone()),
            ("Result".to_string(), game.result.clone()),
        ];
        let (fen, moves, result) = (
            game.start_fen.clone(),
            game.moves.clone(),
            game.result.clone(),
        );
        blocking(move || gm_store::pgn::to_pgn(&headers, &fen, &moves, &result)).await?
    } else {
        game.pgn
    };
    let disposition = HeaderValue::from_str(&format!(
        "attachment; filename=\"grandmentor-game-{id}.pgn\""
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        pgn,
    )
        .into_response())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PgnReq {
    pgn: String,
}

async fn import_games(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(req): ApiJson<PgnReq>,
) -> ApiResult<Json<Vec<gm_store::GameRecord>>> {
    if req.pgn.trim().is_empty() {
        return Err(ApiError::bad_request("`pgn` must not be empty"));
    }
    let store = st.store.clone();
    let content = st.content.localized(lang);
    let games = blocking(move || {
        store.import_pgn_with(&req.pgn, |fen, moves| {
            let start = gm_engine::parse_fen(fen).ok()?;
            detect_opening(&content, &start, moves)
        })
    })
    .await?
    .map_err(|e| ApiError::bad_request(format!("could not import PGN: {e}")))?;
    Ok(Json(games))
}

// ---------------------------------------------------------------------------------------------
// Puzzles
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
struct PuzzleQuery {
    theme: Option<String>,
    min: Option<String>,
    max: Option<String>,
}

async fn user_puzzle_rating(store: &Store) -> u16 {
    match store_op(store, |s| s.get_profile()).await {
        Ok(p) if p.puzzle_rating.is_finite() && p.puzzle_rating > 0.0 => {
            p.puzzle_rating.round().clamp(100.0, 3500.0) as u16
        }
        _ => 1200,
    }
}

async fn puzzle_next(
    State(st): State<AppState>,
    ApiQuery(q): ApiQuery<PuzzleQuery>,
) -> ApiResult<Json<Puzzle>> {
    let min = parse_opt::<u16>("min", &q.min)?;
    let max = parse_opt::<u16>("max", &q.max)?;
    let target = match (min, max) {
        (Some(a), Some(b)) => ((u32::from(a) + u32::from(b)) / 2) as u16,
        _ => user_puzzle_rating(&st.store).await,
    };
    let min = min.unwrap_or(target.saturating_sub(150));
    let max = max.unwrap_or(target.saturating_add(150));
    let recent = st.recent_puzzles.snapshot();
    let p = st
        .puzzles
        .select_next(&st.content, target, min, max, opt_trim(&q.theme), &recent)
        .ok_or_else(|| ApiError::not_found("no puzzles match that filter"))?;
    st.recent_puzzles.insert(p.id.clone());
    Ok(Json(p.clone()))
}

async fn puzzle_daily(State(st): State<AppState>) -> ApiResult<Json<Puzzle>> {
    st.puzzles
        .daily(&st.content, today_utc())
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found("no puzzles available"))
}

async fn puzzle_themes(State(st): State<AppState>) -> Json<Vec<ThemeCount>> {
    Json(st.puzzles.themes.clone())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RushQuery {
    count: Option<String>,
}

async fn puzzle_rush(
    State(st): State<AppState>,
    ApiQuery(q): ApiQuery<RushQuery>,
) -> ApiResult<Json<Vec<Puzzle>>> {
    let count = parse_opt::<usize>("count", &q.count)?
        .unwrap_or(40)
        .clamp(1, 200);
    Ok(Json(
        st.puzzles
            .rush(&st.content, count)
            .into_iter()
            .cloned()
            .collect(),
    ))
}

async fn get_puzzle(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Puzzle>> {
    st.content
        .puzzle(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("puzzle {id:?} not found")))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AttemptReq {
    solved: bool,
    time_ms: u64,
}

async fn puzzle_attempt(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(req): ApiJson<AttemptReq>,
) -> ApiResult<Json<gm_store::PuzzleResult>> {
    let rating = st
        .content
        .puzzle(&id)
        .map(|p| p.rating)
        .ok_or_else(|| ApiError::not_found(format!("puzzle {id:?} not found")))?;
    st.recent_puzzles.insert(id.clone());
    let time_ms = req.time_ms.min(24 * 3600 * 1000);
    let res = store_op(&st.store, move |s| {
        let res = s.record_puzzle_attempt(&id, rating, req.solved, time_ms)?;
        let _ = s.log_activity("puzzle", 1);
        Ok(res)
    })
    .await?;
    Ok(Json(res))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RushReq {
    score: u32,
    /// Rush mode (`"3"`, `"5"`, `"survival"`); when given, the per-mode best is kept too.
    mode: Option<String>,
}

async fn record_rush(
    State(st): State<AppState>,
    ApiJson(req): ApiJson<RushReq>,
) -> ApiResult<Json<Value>> {
    let score = req.score.min(10_000);
    let mode = req.mode.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
    if let Some(m) = &mode {
        if !gm_store::puzzle_profile::valid_mode(m) {
            return Err(ApiError::bad_request(format!("invalid Puzzle Rush mode {m:?}")));
        }
    }
    let (best, mode_bests) = store_op(&st.store, move |s| {
        let best = s.record_rush(score)?;
        // A full mode table (16 modes) only means this mode's best is not kept.
        let mode_bests = match &mode {
            Some(m) => s.record_rush_best(m, score).ok(),
            None => None,
        };
        Ok((best, mode_bests))
    })
    .await?;
    let mut out = json!({ "best": best });
    if let Some((prev, now)) = mode_bests {
        out["mode_best"] = json!(now);
        out["previous_mode_best"] = json!(prev);
    }
    Ok(Json(out))
}

// ---------------------------------------------------------------------------------------------
// Courses & progress
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct LessonSummary {
    id: String,
    title: String,
    summary: String,
}

#[derive(Serialize)]
struct CourseSummary {
    id: String,
    title: String,
    category: String,
    level: String,
    description: String,
    icon: String,
    lesson_count: usize,
    lessons: Vec<LessonSummary>,
}

async fn courses(State(st): State<AppState>, ReqLang(lang): ReqLang) -> Json<Vec<CourseSummary>> {
    Json(
        st.content
            .localized(lang)
            .courses
            .iter()
            .map(|c| CourseSummary {
                id: c.id.clone(),
                title: c.title.clone(),
                category: c.category.clone(),
                level: c.level.clone(),
                description: c.description.clone(),
                icon: c.icon.clone(),
                lesson_count: c.lessons.len(),
                lessons: c
                    .lessons
                    .iter()
                    .map(|l| LessonSummary {
                        id: l.id.clone(),
                        title: l.title.clone(),
                        summary: l.summary.clone(),
                    })
                    .collect(),
            })
            .collect(),
    )
}

async fn course(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<gm_content::Course>> {
    st.content
        .localized(lang)
        .course(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("course {id:?} not found")))
}

async fn get_progress(
    State(st): State<AppState>,
) -> ApiResult<Json<Vec<gm_store::LessonProgress>>> {
    Ok(Json(store_op(&st.store, |s| s.get_progress()).await?))
}

#[derive(Deserialize)]
struct ProgressReq {
    course_id: String,
    lesson_id: String,
    #[serde(default = "default_true")]
    completed: bool,
}

fn default_true() -> bool {
    true
}

async fn set_progress(
    State(st): State<AppState>,
    ApiJson(req): ApiJson<ProgressReq>,
) -> ApiResult<Json<Vec<gm_store::LessonProgress>>> {
    let (c, l) = (
        req.course_id.trim().to_string(),
        req.lesson_id.trim().to_string(),
    );
    if c.is_empty() || l.is_empty() || c.len() > 200 || l.len() > 200 {
        return Err(ApiError::bad_request(
            "`course_id` and `lesson_id` are required",
        ));
    }
    let completed = req.completed;
    let list = store_op(&st.store, move |s| {
        s.set_lesson_progress(&c, &l, completed)?;
        if completed {
            let _ = s.log_activity("lesson", 1);
        }
        s.get_progress()
    })
    .await?;
    Ok(Json(list))
}

// ---------------------------------------------------------------------------------------------
// Openings & endgames
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpeningsQuery {
    q: Option<String>,
    side: Option<String>,
    level: Option<String>,
}

/// Filter on the English source entry `o`; the search text also matches the localized name
/// and family `loc` (so "italiana" and "Italian" both find the Italian Game).
fn opening_matches(
    o: &Opening,
    loc: &Opening,
    q: Option<&str>,
    side: Option<&str>,
    level: Option<&str>,
) -> bool {
    if let Some(side) = side {
        if !o.side.eq_ignore_ascii_case(side) {
            return false;
        }
    }
    if let Some(level) = level {
        if !o.level.eq_ignore_ascii_case(level) {
            return false;
        }
    }
    match q {
        None => true,
        Some(q) => {
            let q = q.to_lowercase();
            o.name.to_lowercase().contains(&q)
                || o.family.to_lowercase().contains(&q)
                || loc.name.to_lowercase().contains(&q)
                || loc.family.to_lowercase().contains(&q)
                || o.eco.to_lowercase().starts_with(&q)
                || o.moves.to_lowercase().starts_with(&q)
        }
    }
}

async fn openings(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<OpeningsQuery>,
) -> Json<Vec<Opening>> {
    let (qs, side, level) = (opt_trim(&q.q), opt_trim(&q.side), opt_trim(&q.level));
    let side = side.filter(|s| !s.eq_ignore_ascii_case("all") && !s.eq_ignore_ascii_case("both"));
    let level = level.filter(|s| !s.eq_ignore_ascii_case("all"));
    let loc = st.content.localized(lang);
    // Views keep the source order (overlays never add, drop or reorder entries).
    let mut out: Vec<Opening> = st
        .content
        .openings
        .iter()
        .zip(&loc.openings)
        .filter(|(o, l)| opening_matches(o, l, qs, side, level))
        .map(|(_, l)| l.clone())
        .collect();
    out.sort_by(|a, b| {
        b.popularity
            .cmp(&a.popularity)
            .then_with(|| a.name.cmp(&b.name))
    });
    Json(out)
}

async fn opening(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Opening>> {
    st.content
        .localized(lang)
        .opening(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("opening {id:?} not found")))
}

async fn opening_lookup(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<FenQuery>,
) -> ApiResult<Json<Option<OpeningMatch>>> {
    let fen = opt_trim(&q.fen).ok_or_else(|| ApiError::bad_request("missing `fen`"))?;
    let fen = to_fen(&parse_position(fen)?);
    let key = format!("{lang}|{}", fen_key(&fen));
    if let Some(hit) = st.opening_cache.get(&key) {
        return Ok(Json(hit));
    }
    let content = st.content.localized(lang);
    let m = blocking(move || content.lookup_opening(&fen)).await?;
    st.opening_cache.insert(key, m.clone());
    Ok(Json(m))
}

async fn endgames(State(st): State<AppState>, ReqLang(lang): ReqLang) -> Json<Vec<gm_content::EndgameDrill>> {
    Json(st.content.localized(lang).endgames.clone())
}

async fn endgame(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<gm_content::EndgameDrill>> {
    st.content
        .localized(lang)
        .endgame(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("endgame {id:?} not found")))
}

// ---------------------------------------------------------------------------------------------
// Profile & stats
// ---------------------------------------------------------------------------------------------

async fn get_profile(State(st): State<AppState>) -> ApiResult<Json<gm_store::Profile>> {
    Ok(Json(store_op(&st.store, |s| s.get_profile()).await?))
}

async fn update_profile(
    State(st): State<AppState>,
    ApiJson(mut p): ApiJson<ProfilePatch>,
) -> ApiResult<Json<gm_store::Profile>> {
    if let Some(name) = &p.name {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 40 {
            return Err(ApiError::bad_request("name must be 1-40 characters"));
        }
        p.name = Some(name.to_string());
    }
    if let Some(a) = &p.avatar {
        if a.chars().count() > 16 {
            return Err(ApiError::bad_request("avatar too long"));
        }
    }
    if let Some(s) = &p.settings_json {
        if s.len() > 64 * 1024 {
            return Err(ApiError::bad_request("settings too large"));
        }
        if serde_json::from_str::<Value>(s).is_err() {
            return Err(ApiError::bad_request("settings_json must be valid JSON"));
        }
    }
    Ok(Json(
        store_op(&st.store, move |s| s.update_profile(&p)).await?,
    ))
}

async fn stats(State(st): State<AppState>) -> ApiResult<Json<gm_store::Stats>> {
    Ok(Json(store_op(&st.store, |s| s.stats()).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_clamped() {
        let l = clamp_limits(None, None, None, None, 10_000, 1_000);
        assert_eq!(l.movetime_ms, Some(1_000));
        assert_eq!(l.multipv, 1);
        let l = clamp_limits(Some(200), None, None, Some(50), 30_000, 3_000);
        assert_eq!(l.depth, Some(60));
        assert_eq!(l.movetime_ms, Some(30_000));
        assert_eq!(l.multipv, MAX_MULTIPV);
        let l = clamp_limits(None, Some(999_999), None, Some(0), 30_000, 3_000);
        assert_eq!(l.movetime_ms, Some(30_000));
        assert_eq!(l.multipv, 1);
    }

    #[test]
    fn clip_is_char_safe() {
        assert_eq!(clip("héllo", 2), "hé");
        assert_eq!(clip("hi", 10), "hi");
    }
}
