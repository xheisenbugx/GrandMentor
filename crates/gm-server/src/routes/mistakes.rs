//! Learn from your mistakes: spaced-repetition puzzles from the user's own games (`/api/mistakes*`).
//!
//! Cards are created from game reviews: every move by the user's side classified as a mistake,
//! miss or blunder (with a clearly better engine move) becomes a "find the better move" card.
//! Ingestion happens when a review is saved to a game ([`ingest_review`], called from `api.rs`)
//! and through the bounded backfill `POST /api/mistakes/sync`. Scheduling lives in
//! `gm_store::srs`.

use axum::extract::State;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shakmaty::{Chess, Position};

use gm_analysis::{Classification, GameReview, MoveReview};
use gm_content::Lang;
use gm_engine::Score;
use gm_store::srs::{AttemptOutcome, MistakeCard, MistakeSummary, NewMistake};
use gm_store::GameRecord;

use crate::api::{blocking, store_op};
use crate::error::{ApiError, ApiJson, ApiPath, ApiQuery, ApiResult};
use crate::lang::ReqLang;
use crate::state::AppState;

/// Max games scanned by one `POST /api/mistakes/sync`.
pub const SYNC_MAX_GAMES: u32 = 100;
/// Minimum win-chance loss (percentage points) for a move to become a card.
pub const MIN_LOSS: f32 = 10.0;
/// Max cards taken from a single game (worst first).
pub const MAX_CARDS_PER_GAME: usize = 12;
/// Default / max list page size.
const DEFAULT_PAGE: u32 = 20;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/mistakes", get(list))
        .route("/mistakes/summary", get(summary))
        .route("/mistakes/next", get(next))
        .route("/mistakes/sync", post(sync))
        .route("/mistakes/:id", delete(remove))
        .route("/mistakes/:id/attempt", post(attempt))
}

// ---------------------------------------------------------------------------------------------
// Building cards from a review
// ---------------------------------------------------------------------------------------------

fn fullmove(fen: &str) -> u32 {
    fen.split_whitespace().nth(5).and_then(|s| s.parse().ok()).unwrap_or(1).clamp(1, 10_000)
}

/// "opening" | "middlegame" | "endgame" from the position's material and move number.
pub(crate) fn phase_of(fen: &str) -> &'static str {
    let placement = fen.split_whitespace().next().unwrap_or("");
    let (mut material, mut queens) = (0u32, 0u32);
    for c in placement.chars() {
        match c.to_ascii_lowercase() {
            'q' => {
                material += 9;
                queens += 1;
            }
            'r' => material += 5,
            'b' | 'n' => material += 3,
            _ => {}
        }
    }
    if material <= 26 || (queens == 0 && material <= 30) {
        "endgame"
    } else if fullmove(fen) <= 10 {
        "opening"
    } else {
        "middlegame"
    }
}

/// UCI solution line: the best move plus the engine continuation while it stays forcing
/// (the opponent's reply is its only legal move, or the line is a forced mate). Odd length.
fn solution_line(m: &MoveReview, user: shakmaty::Color) -> Vec<String> {
    let mut out = vec![m.best_move_uci.clone()];
    let Ok(mut pos) = gm_engine::parse_fen(&m.fen_before) else {
        return out;
    };
    let mut ucis: Vec<String> = Vec::new();
    for san in m.best_line_san.iter().take(gm_store::srs::MAX_SOLUTION_PLIES) {
        let Ok(mv) = gm_engine::san_to_move(&pos, san) else { break };
        ucis.push(gm_engine::move_to_uci(&mv));
        pos.play_unchecked(&mv);
    }
    if ucis.first() != Some(&m.best_move_uci) {
        return out;
    }
    // Forced mate for the user: keep the whole mating line (bounded).
    let mate_plies = match m.eval_before.for_side(user) {
        Score::Mate(n) if n > 0 => Some((2 * n - 1) as usize),
        _ => None,
    };
    if let Some(n) = mate_plies {
        if n <= ucis.len() && n <= gm_store::srs::MAX_SOLUTION_PLIES {
            return ucis[..n].to_vec();
        }
    }
    // Otherwise extend while the opponent has a single legal reply.
    let Ok(mut pos) = gm_engine::parse_fen(&m.fen_before) else {
        return out;
    };
    let mut i = 0;
    while i + 2 < ucis.len() {
        let Ok(mv) = gm_engine::uci_to_move(&pos, &ucis[i]) else { break };
        pos.play_unchecked(&mv);
        if pos.legal_moves().len() != 1 {
            break;
        }
        let Ok(reply) = gm_engine::uci_to_move(&pos, &ucis[i + 1]) else { break };
        pos.play_unchecked(&reply);
        if gm_engine::uci_to_move(&pos, &ucis[i + 2]).is_err() {
            break;
        }
        out.push(ucis[i + 1].clone());
        out.push(ucis[i + 2].clone());
        i += 2;
    }
    out
}

/// Cards for every clear error the user made in a reviewed game (worst first, bounded).
pub(crate) fn cards_from_review(game: &GameRecord, review: &GameReview, lang: Lang) -> Vec<NewMistake> {
    let user = match game.user_color.as_deref() {
        Some("white") => shakmaty::Color::White,
        Some("black") => shakmaty::Color::Black,
        _ => return Vec::new(),
    };
    let user_str = if user.is_white() { "white" } else { "black" };
    let opponent = if user.is_white() { &game.black } else { &game.white };
    let mut picks: Vec<&MoveReview> = review
        .moves
        .iter()
        .filter(|m| {
            m.color == user_str
                && matches!(
                    m.classification,
                    Classification::Mistake | Classification::Miss | Classification::Blunder
                )
                && m.win_chance_loss >= MIN_LOSS
                && !m.best_move_uci.is_empty()
                && m.best_move_uci != m.uci
        })
        .collect();
    picks.sort_by(|a, b| b.win_chance_loss.total_cmp(&a.win_chance_loss));
    picks.truncate(MAX_CARDS_PER_GAME);
    picks.sort_by_key(|m| m.ply);

    picks
        .into_iter()
        .filter_map(|m| {
            // The position must be legal and the stored moves must be playable in it.
            let pos: Chess = gm_engine::parse_fen(&m.fen_before).ok()?;
            if pos.turn() != user || gm_engine::uci_to_move(&pos, &m.best_move_uci).is_err() {
                return None;
            }
            let prev = m.ply.checked_sub(2).and_then(|i| review.moves.get(i));
            let (prev_fen, prev_uci) = match prev {
                Some(p) if p.fen_after == m.fen_before => (Some(p.fen_before.clone()), Some(p.uci.clone())),
                _ => (None, None),
            };
            Some(NewMistake {
                fen: m.fen_before.clone(),
                prev_fen,
                prev_uci,
                played_uci: m.uci.clone(),
                played_san: m.san.clone(),
                best_uci: m.best_move_uci.clone(),
                best_san: m.best_move_san.clone(),
                solution: solution_line(m, user),
                game_id: Some(game.id),
                ply: u32::try_from(m.ply).unwrap_or(0),
                move_number: fullmove(&m.fen_before),
                color: user_str.to_string(),
                classification: m.classification.as_str().to_string(),
                phase: phase_of(&m.fen_before).to_string(),
                opponent: opponent.clone(),
                bot_id: game.bot_id.clone(),
                explanation: m.explanation.clone(),
                lang: lang.code().to_string(),
                win_chance_loss: m.win_chance_loss,
            })
        })
        .collect()
}

/// Adds the user's mistakes from a freshly saved review to the deck and marks the game scanned.
/// Best effort: failures are logged, never surfaced. Returns the number of new cards.
pub(crate) async fn ingest_review(st: &AppState, game: &GameRecord, review: &GameReview, lang: Lang) -> usize {
    let cards = cards_from_review(game, review, lang);
    let id = game.id;
    let res = store_op(&st.store, move |s| {
        let added = s.add_mistakes(&cards)?;
        s.mark_game_scanned(id)?;
        Ok(added)
    })
    .await;
    match res {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("could not add mistakes from game {id}: {}", e.message);
            0
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

async fn summary(State(st): State<AppState>) -> ApiResult<Json<MistakeSummary>> {
    Ok(Json(store_op(&st.store, |s| s.mistake_summary()).await?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NextQuery {
    /// Skip this card (the one just answered) when another is available.
    exclude: Option<i64>,
}

#[derive(Serialize)]
struct NextResp {
    card: Option<MistakeCard>,
    /// True when `card` is due now (false = the soonest upcoming card, for early practice).
    due: bool,
    summary: MistakeSummary,
}

/// The card's coach explanation in `lang` (re-localized from the stored game review when the
/// card was created in another language). Empty when unavailable.
fn localized_explanation(st: &AppState, card: &MistakeCard, lang: Lang) -> String {
    if card.lang == lang.code() {
        return card.explanation.clone();
    }
    let Some(game_id) = card.game_id else { return String::new() };
    let Ok(Some(game)) = st.store.get_game(game_id) else { return String::new() };
    let Some((review, stored_lang)) = game.review_json.as_deref().and_then(gm_analysis::from_stored_json) else {
        return String::new();
    };
    let review = if stored_lang == lang {
        review
    } else {
        gm_analysis::relocalize(&review, &st.content, lang)
    };
    review
        .moves
        .get((card.ply as usize).saturating_sub(1))
        .filter(|m| m.fen_before == card.fen && m.uci == card.played_uci)
        .map(|m| m.explanation.clone())
        .unwrap_or_default()
}

async fn next(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<NextQuery>,
) -> ApiResult<Json<NextResp>> {
    let st2 = st.clone();
    let resp = blocking(move || -> anyhow::Result<NextResp> {
        let mut card = st2.store.next_mistake(q.exclude)?;
        if let Some(c) = card.as_mut() {
            c.explanation = localized_explanation(&st2, c, lang);
            c.lang = lang.code().to_string();
        }
        let summary = st2.store.mistake_summary()?;
        let due = card.as_ref().is_some_and(|c| c.due_in_secs <= 0);
        Ok(NextResp { card, due, summary })
    })
    .await?
    .map_err(ApiError::from)?;
    Ok(Json(resp))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AttemptReq {
    solved: bool,
    time_ms: Option<u64>,
}

async fn attempt(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(req): ApiJson<AttemptReq>,
) -> ApiResult<Json<AttemptOutcome>> {
    let time_ms = req.time_ms.unwrap_or(0).min(24 * 3600 * 1000);
    let out = store_op(&st.store, move |s| {
        let out = s.attempt_mistake(id, req.solved, time_ms)?;
        if out.is_some() {
            if let Err(e) = s.log_activity("mistake_review", 1) {
                tracing::warn!("could not log mistake_review activity: {e:#}");
            }
        }
        Ok(out)
    })
    .await?
    .ok_or_else(|| ApiError::not_found(format!("mistake card {id} not found")))?;
    Ok(Json(out))
}

async fn remove(State(st): State<AppState>, ApiPath(id): ApiPath<i64>) -> ApiResult<Json<Value>> {
    let ok = store_op(&st.store, move |s| s.remove_mistake(id)).await?;
    if !ok {
        return Err(ApiError::not_found(format!("mistake card {id} not found")));
    }
    let summary = store_op(&st.store, |s| s.mistake_summary()).await?;
    Ok(Json(json!({ "ok": true, "summary": summary })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ListQuery {
    /// "due" | "learning" | "graduated" | "all" (default)
    filter: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn list(State(st): State<AppState>, ApiQuery(q): ApiQuery<ListQuery>) -> ApiResult<Json<Value>> {
    let filter = q.filter.unwrap_or_default();
    if !filter.is_empty() && !["all", "due", "learning", "graduated"].contains(&filter.as_str()) {
        return Err(ApiError::bad_request("filter must be one of: all, due, learning, graduated"));
    }
    let limit = q.limit.unwrap_or(DEFAULT_PAGE).clamp(1, gm_store::srs::MAX_PAGE);
    let offset = q.offset.unwrap_or(0);
    let (items, summary) = store_op(&st.store, move |s| {
        Ok((s.list_mistakes(&filter, limit, offset)?, s.mistake_summary()?))
    })
    .await?;
    Ok(Json(json!({ "items": items, "summary": summary, "limit": limit, "offset": offset })))
}

#[derive(Serialize)]
struct SyncResp {
    scanned: usize,
    added: usize,
    /// More reviewed games remain to be scanned (call again).
    more: bool,
    summary: MistakeSummary,
}

/// Backfill: scan reviewed games that were never scanned (bounded per call).
async fn sync(State(st): State<AppState>) -> ApiResult<Json<SyncResp>> {
    let store = st.store.clone();
    let resp = blocking(move || -> anyhow::Result<SyncResp> {
        let ids = store.unscanned_reviewed_games(SYNC_MAX_GAMES + 1)?;
        let more = ids.len() > SYNC_MAX_GAMES as usize;
        let (mut scanned, mut added) = (0, 0);
        for id in ids.into_iter().take(SYNC_MAX_GAMES as usize) {
            if let Some(game) = store.get_game(id)? {
                if let Some((review, lang)) = game.review_json.as_deref().and_then(gm_analysis::from_stored_json) {
                    added += store.add_mistakes(&cards_from_review(&game, &review, lang))?;
                }
            }
            store.mark_game_scanned(id)?;
            scanned += 1;
        }
        Ok(SyncResp { scanned, added, more, summary: store.mistake_summary()? })
    })
    .await?
    .map_err(ApiError::from)?;
    Ok(Json(resp))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(ply: usize, color: &str, fen_before: &str, uci: &str, best: &str, cls: Classification, loss: f32) -> MoveReview {
        let pos = gm_engine::parse_fen(fen_before).unwrap();
        let mut after = pos.clone();
        after.play_unchecked(&gm_engine::uci_to_move(&pos, uci).unwrap());
        MoveReview {
            ply,
            san: gm_engine::move_to_san(&pos, &gm_engine::uci_to_move(&pos, uci).unwrap()),
            uci: uci.into(),
            color: color.into(),
            fen_before: fen_before.into(),
            fen_after: gm_engine::to_fen(&after),
            best_move_uci: best.into(),
            best_move_san: gm_engine::move_to_san(&pos, &gm_engine::uci_to_move(&pos, best).unwrap()),
            best_line_san: vec![gm_engine::move_to_san(&pos, &gm_engine::uci_to_move(&pos, best).unwrap())],
            classification: cls,
            win_chance_loss: loss,
            explanation: "x".into(),
            ..Default::default()
        }
    }

    #[test]
    fn phases() {
        assert_eq!(phase_of(gm_engine::START_FEN), "opening");
        assert_eq!(phase_of("6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 30"), "endgame");
        assert_eq!(
            phase_of("r1bq1rk1/pppp1ppp/2n2n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 15"),
            "middlegame"
        );
    }

    #[test]
    fn picks_only_the_users_clear_errors() {
        let start = gm_engine::START_FEN;
        let m1 = mv(1, "white", start, "f2f3", "e2e4", Classification::Blunder, 25.0);
        let m2 = mv(2, "black", &m1.fen_after.clone(), "e7e5", "e7e5", Classification::Best, 0.0);
        let m3 = mv(3, "white", &m2.fen_after.clone(), "g2g4", "e2e4", Classification::Inaccuracy, 8.0);
        let m4 = mv(4, "black", &m3.fen_after.clone(), "d8h4", "d8h4", Classification::Best, 0.0);
        let review = GameReview { moves: vec![m1, m2, m3, m4], ..Default::default() };
        let game = GameRecord {
            id: 7,
            white: "Me".into(),
            black: "Bot Bob".into(),
            user_color: Some("white".into()),
            ..Default::default()
        };
        let cards = cards_from_review(&game, &review, Lang::Es);
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!((c.ply, c.best_uci.as_str(), c.played_san.as_str()), (1, "e2e4", "f3"));
        assert_eq!(c.opponent, "Bot Bob");
        assert_eq!(c.lang, "es");
        assert!(c.prev_fen.is_none());
        // Black's side: nothing to learn.
        let game_b = GameRecord { user_color: Some("black".into()), ..game.clone() };
        assert!(cards_from_review(&game_b, &review, Lang::En).is_empty());
        let game_none = GameRecord { user_color: None, ..game };
        assert!(cards_from_review(&game_none, &review, Lang::En).is_empty());
    }

    #[test]
    fn forced_mate_line_is_kept() {
        // White to move: Qxf7# style. Back-rank mate in 1 with the rook.
        let fen = "6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1";
        let mut m = mv(9, "white", fen, "h2h3", "a1a8", Classification::Miss, 40.0);
        m.eval_before = Score::Mate(1);
        let line = solution_line(&m, shakmaty::Color::White);
        assert_eq!(line, vec!["a1a8".to_string()]);
    }
}
