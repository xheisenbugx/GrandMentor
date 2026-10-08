//! Opening repertoire builder and drills (`/api/repertoire*`). See docs/CONTRACT.md "Repertoire".

use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use gm_content::Lang;
use gm_store::repertoire::{AddOutcome, Deviation, RepError, RepNode, RepSummary, RepTree, ReviewOutcome, Side};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::store_op;
use crate::error::{ApiError, ApiJson, ApiPath, ApiQuery, ApiResult};
use crate::lang::ReqLang;
use crate::state::AppState;

/// Max moves accepted in one `POST /repertoire/lines`.
const MAX_LINE_MOVES: usize = gm_store::repertoire::MAX_PLY;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/repertoire", get(tree).delete(clear))
        .route("/repertoire/summary", get(summary))
        .route("/repertoire/nodes", post(add_node))
        .route(
            "/repertoire/nodes/:id",
            put(update_node).patch(update_node).delete(delete_node),
        )
        .route("/repertoire/lines", post(add_line))
        .route("/repertoire/drill/next", get(drill_next))
        .route("/repertoire/drill/attempt", post(drill_attempt))
        .route("/repertoire/deviations", get(deviations))
        .route("/repertoire/starters", get(starters))
        .route("/repertoire/starters/:id", post(apply_starter))
}

// ---------------------------------------------------------------------------------------------
// Starter repertoires (built from data/openings.json lines)
// ---------------------------------------------------------------------------------------------

/// (id, side, opening ids). Titles/blurbs live in the frontend locales (`repertoire.starters.*`).
pub const STARTERS: &[(&str, Side, &[&str])] = &[
    (
        "italian",
        Side::White,
        &[
            "giuoco-pianissimo",
            "italian-two-knights-d3",
            "italian-game",
            "sicilian-alapin",
            "french-advance",
            "caro-kann-advance",
            "scandinavian-qa5",
            "pirc-classical",
            "modern-defense-standard",
            "alekhine-modern",
            "philidor-exchange",
            "petrov-steinitz",
        ],
    ),
    (
        "london",
        Side::White,
        &["london-system", "london-system-indian", "dutch-defense", "old-benoni", "englund-gambit"],
    ),
    (
        "caro-kann-qgd",
        Side::Black,
        &[
            "caro-kann-classical",
            "caro-kann-advance-short",
            "caro-kann-exchange",
            "caro-kann-panov",
            "caro-kann-two-knights",
            "caro-kann-fantasy",
            "qgd-normal",
            "qgd-exchange",
            "queens-pawn-zukertort",
            "london-system",
            "english-agincourt",
            "reti-main",
        ],
    ),
    (
        "open-game-qgd",
        Side::Black,
        &[
            "giuoco-piano-main",
            "scotch-classical",
            "ruy-lopez-closed",
            "four-knights-game",
            "vienna-falkbeer",
            "kings-gambit-accepted",
            "bishops-opening-berlin",
            "qgd-normal",
            "qgd-exchange",
            "queens-pawn-zukertort",
            "london-system",
        ],
    ),
];

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Pick the message for `lang` from `[en, es, pt, fr, de]`.
fn msg(lang: Lang, s: [&str; 5]) -> String {
    match lang {
        Lang::En => s[0],
        Lang::Es => s[1],
        Lang::Pt => s[2],
        Lang::Fr => s[3],
        Lang::De => s[4],
    }
    .to_string()
}

fn parse_side(lang: Lang, s: &str) -> ApiResult<Side> {
    Side::parse(s).ok_or_else(|| {
        ApiError::bad_request(msg(
            lang,
            [
                "`side` must be \"white\" or \"black\"",
                "`side` debe ser \"white\" o \"black\"",
                "`side` deve ser \"white\" ou \"black\"",
                "`side` doit valoir \"white\" ou \"black\"",
                "`side` muss \"white\" oder \"black\" sein",
            ],
        ))
    })
}

fn rep_error(lang: Lang, e: RepError) -> ApiError {
    match e {
        RepError::NotFound => ApiError::not_found(msg(
            lang,
            [
                "repertoire move not found",
                "No se encontró esa jugada del repertorio",
                "Esse lance do repertório não foi encontrado",
                "Ce coup du répertoire est introuvable",
                "Dieser Repertoirezug wurde nicht gefunden",
            ],
        )),
        RepError::NoMoves => ApiError::bad_request(msg(
            lang,
            ["no moves given", "No se indicó ninguna jugada", "Nenhum lance foi informado", "Aucun coup n'a été indiqué", "Es wurden keine Züge angegeben"],
        )),
        RepError::TooDeep => ApiError::bad_request(match lang {
            Lang::En => format!("line too long (max {} plies)", gm_store::repertoire::MAX_PLY),
            Lang::Es => format!("Línea demasiado larga (máximo {} medias jugadas)", gm_store::repertoire::MAX_PLY),
            Lang::Pt => format!("Linha longa demais (máximo de {} meios-lances)", gm_store::repertoire::MAX_PLY),
            Lang::Fr => format!("Ligne trop longue ({} demi-coups maximum)", gm_store::repertoire::MAX_PLY),
            Lang::De => format!("Linie zu lang (höchstens {} Halbzüge)", gm_store::repertoire::MAX_PLY),
        }),
        RepError::Full => ApiError::bad_request(match lang {
            Lang::En => format!(
                "your repertoire is full (max {} moves per side)",
                gm_store::repertoire::MAX_NODES_PER_SIDE
            ),
            Lang::Es => format!(
                "Tu repertorio está lleno (máximo {} jugadas por color)",
                gm_store::repertoire::MAX_NODES_PER_SIDE
            ),
            Lang::Pt => format!(
                "Seu repertório está cheio (máximo de {} lances por cor)",
                gm_store::repertoire::MAX_NODES_PER_SIDE
            ),
            Lang::Fr => format!(
                "Ton répertoire est plein ({} coups maximum par couleur)",
                gm_store::repertoire::MAX_NODES_PER_SIDE
            ),
            Lang::De => format!(
                "Dein Repertoire ist voll (höchstens {} Züge pro Farbe)",
                gm_store::repertoire::MAX_NODES_PER_SIDE
            ),
        }),
        RepError::Illegal(i, m) => ApiError::bad_request(match lang {
            Lang::En => format!("move {i} is illegal: {m}"),
            Lang::Es => format!("La jugada {i} es ilegal: {m}"),
            Lang::Pt => format!("O lance {i} é ilegal: {m}"),
            Lang::Fr => format!("Le coup {i} est illégal : {m}"),
            Lang::De => format!("Zug {i} ist illegal: {m}"),
        }),
        RepError::WrongSide => ApiError::bad_request(msg(
            lang,
            [
                "that move belongs to the other side's repertoire",
                "Esa jugada pertenece al repertorio del otro color",
                "Esse lance pertence ao repertório da outra cor",
                "Ce coup appartient au répertoire de l'autre couleur",
                "Dieser Zug gehört zum Repertoire der anderen Farbe",
            ],
        )),
        RepError::NotACard => ApiError::bad_request(msg(
            lang,
            [
                "only your own moves can be drilled",
                "Solo se entrenan tus propias jugadas",
                "Só os seus próprios lances podem ser treinados",
                "Seuls tes propres coups peuvent être entraînés",
                "Nur deine eigenen Züge können trainiert werden",
            ],
        )),
    }
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
struct SideQuery {
    side: String,
}

async fn tree(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<SideQuery>,
) -> ApiResult<Json<RepTree>> {
    let side = parse_side(lang, &q.side)?;
    let now = now_secs();
    store_op(&st.store, move |s| s.repertoire_tree(side, now)).await.map(Json)
}

async fn clear(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<SideQuery>,
) -> ApiResult<Json<Value>> {
    let side = parse_side(lang, &q.side)?;
    let n = store_op(&st.store, move |s| s.repertoire_clear(side)).await?;
    Ok(Json(json!({ "deleted": n })))
}

async fn summary(State(st): State<AppState>) -> ApiResult<Json<RepSummary>> {
    let now = now_secs();
    store_op(&st.store, move |s| s.repertoire_summary(now)).await.map(Json)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AddNodeBody {
    side: String,
    /// 0 / missing = from the initial position.
    parent_id: i64,
    uci: String,
    note: Option<String>,
    replace: bool,
}

async fn add_node(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(b): ApiJson<AddNodeBody>,
) -> ApiResult<Json<AddOutcome>> {
    let side = parse_side(lang, &b.side)?;
    let now = now_secs();
    let moves = vec![b.uci.chars().take(8).collect::<String>()];
    let parent = b.parent_id.max(0);
    let note = b.note;
    let out = store_op(&st.store, move |s| {
        let r = s.repertoire_add_line(side, parent, &moves, b.replace, now)?;
        if let (Ok(o), Some(n)) = (&r, note.as_deref()) {
            if let Some(&last) = o.path.last() {
                s.repertoire_set_note(last, n, now)?;
            }
        }
        Ok(r)
    })
    .await?;
    out.map(Json).map_err(|e| rep_error(lang, e))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AddLineBody {
    side: String,
    parent_id: i64,
    moves: Vec<String>,
    replace: bool,
}

async fn add_line(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(b): ApiJson<AddLineBody>,
) -> ApiResult<Json<AddOutcome>> {
    let side = parse_side(lang, &b.side)?;
    if b.moves.len() > MAX_LINE_MOVES {
        return Err(rep_error(lang, RepError::TooDeep));
    }
    let moves: Vec<String> = b.moves.iter().map(|m| m.chars().take(8).collect()).collect();
    let now = now_secs();
    let parent = b.parent_id.max(0);
    let out = store_op(&st.store, move |s| s.repertoire_add_line(side, parent, &moves, b.replace, now)).await?;
    out.map(Json).map_err(|e| rep_error(lang, e))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NoteBody {
    note: String,
}

async fn update_node(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<NoteBody>,
) -> ApiResult<Json<RepNode>> {
    if b.note.chars().count() > gm_store::repertoire::MAX_NOTE {
        return Err(ApiError::bad_request(match lang {
            Lang::En => format!("note too long (max {} characters)", gm_store::repertoire::MAX_NOTE),
            Lang::Es => format!("Nota demasiado larga (máximo {} caracteres)", gm_store::repertoire::MAX_NOTE),
            Lang::Pt => format!("Nota longa demais (máximo de {} caracteres)", gm_store::repertoire::MAX_NOTE),
            Lang::Fr => format!("Note trop longue ({} caractères maximum)", gm_store::repertoire::MAX_NOTE),
            Lang::De => format!("Notiz zu lang (höchstens {} Zeichen)", gm_store::repertoire::MAX_NOTE),
        }));
    }
    let now = now_secs();
    store_op(&st.store, move |s| s.repertoire_set_note(id, &b.note, now))
        .await?
        .map(Json)
        .ok_or_else(|| rep_error(lang, RepError::NotFound))
}

async fn delete_node(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiPath(id): ApiPath<i64>,
) -> ApiResult<Json<Value>> {
    let n = store_op(&st.store, move |s| s.repertoire_delete(id)).await?;
    if n == 0 {
        return Err(rep_error(lang, RepError::NotFound));
    }
    Ok(Json(json!({ "deleted": n })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DrillQuery {
    side: Option<String>,
    any: Option<String>,
}

async fn drill_next(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<DrillQuery>,
) -> ApiResult<Json<Value>> {
    let side = match q.side.as_deref().map(str::trim).filter(|s| !s.is_empty() && *s != "all") {
        Some(s) => Some(parse_side(lang, s)?),
        None => None,
    };
    let any = matches!(q.any.as_deref().map(str::trim), Some("1" | "true" | "yes"));
    let now = now_secs();
    let seed: u64 = rand::random();
    let line = store_op(&st.store, move |s| s.repertoire_drill_next(side, any, seed, now)).await?;
    Ok(Json(json!({ "line": line })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AttemptBody {
    node_id: i64,
    uci: String,
}

async fn drill_attempt(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(b): ApiJson<AttemptBody>,
) -> ApiResult<Json<ReviewOutcome>> {
    let uci: String = b.uci.chars().take(8).collect();
    let now = now_secs();
    let out = store_op(&st.store, move |s| s.repertoire_review(b.node_id, &uci, now)).await?;
    out.map(Json).map_err(|e| rep_error(lang, e))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DeviationQuery {
    limit: Option<u32>,
}

async fn deviations(
    State(st): State<AppState>,
    ApiQuery(q): ApiQuery<DeviationQuery>,
) -> ApiResult<Json<Vec<Deviation>>> {
    let limit = q.limit.unwrap_or(8);
    let now = now_secs();
    store_op(&st.store, move |s| s.repertoire_deviations(limit, now)).await.map(Json)
}

#[derive(Serialize)]
struct StarterOpening {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct Starter {
    id: &'static str,
    side: Side,
    openings: Vec<StarterOpening>,
    lines: usize,
}

async fn starters(State(st): State<AppState>, ReqLang(lang): ReqLang) -> Json<Vec<Starter>> {
    let content = st.content.localized(lang);
    let out = STARTERS
        .iter()
        .map(|(id, side, ids)| {
            let openings: Vec<StarterOpening> = ids
                .iter()
                .filter_map(|oid| content.opening(oid))
                .map(|o| StarterOpening { id: o.id.clone(), name: o.name.clone() })
                .collect();
            Starter { id, side: *side, lines: openings.len(), openings }
        })
        .collect();
    Json(out)
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
struct StarterOutcome {
    side: String,
    added: u32,
    lines: u32,
    /// Lines skipped because they clash with moves already in your repertoire.
    skipped: u32,
}

async fn apply_starter(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<StarterOutcome>> {
    let Some((_, side, ids)) = STARTERS.iter().find(|(sid, _, _)| *sid == id) else {
        return Err(ApiError::not_found(match lang {
            Lang::En => format!("starter repertoire {id:?} not found"),
            Lang::Es => format!("No se encontró el repertorio inicial {id:?}"),
            Lang::Pt => format!("O repertório inicial {id:?} não foi encontrado"),
            Lang::Fr => format!("Répertoire de départ {id:?} introuvable"),
            Lang::De => format!("Start-Repertoire {id:?} nicht gefunden"),
        }));
    };
    let side = *side;
    let lines: Vec<Vec<String>> = ids
        .iter()
        .filter_map(|oid| st.content.opening(oid))
        .map(|o| o.uci.clone())
        .filter(|u| !u.is_empty())
        .collect();
    let now = now_secs();
    let out = store_op(&st.store, move |s| {
        let mut out = StarterOutcome { side: side.as_str().into(), ..Default::default() };
        for line in &lines {
            match s.repertoire_add_line(side, 0, line, false, now)? {
                Ok(r) if r.conflict.is_none() => {
                    out.added += r.added;
                    out.lines += 1;
                }
                Ok(_) => out.skipped += 1,
                Err(RepError::Full) => {
                    out.skipped += 1;
                    break;
                }
                Err(_) => out.skipped += 1,
            }
        }
        Ok(out)
    })
    .await?;
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{header, Method, Request, StatusCode};
    use tower::ServiceExt as _;

    fn content() -> gm_content::Content {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        gm_content::Content::load(&dir).expect("content")
    }

    #[test]
    fn starter_lines_exist_and_never_clash() {
        let content = content();
        for (id, side, ids) in STARTERS {
            let store = gm_store::Store::open_in_memory().unwrap();
            for oid in *ids {
                let o = content.opening(oid).unwrap_or_else(|| panic!("{id}: unknown opening {oid}"));
                let r = store.repertoire_add_line(*side, 0, &o.uci, false, 0).unwrap().unwrap();
                assert!(r.conflict.is_none(), "{id}: {oid} clashes with an earlier line: {:?}", r.conflict);
            }
        }
    }

    fn app() -> axum::Router {
        let state = AppState::new(
            Arc::new(content()),
            gm_engine::EnginePool::new(1, 4),
            gm_store::Store::open_in_memory().unwrap(),
            Arc::new(gm_mentor::Mentor::from_env()),
        );
        let web = std::env::temp_dir();
        crate::app(state, &web)
    }

    async fn call(app: &axum::Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn crud_drill_and_errors() {
        let app = app();
        let (s, v) = call(&app, Method::GET, "/api/repertoire?side=white", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["nodes"].as_array().unwrap().len(), 0);
        assert_eq!(v["max_nodes"], 5000);

        let (s, _) = call(&app, Method::GET, "/api/repertoire?side=green", None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        let (s, v) = call(
            &app,
            Method::POST,
            "/api/repertoire/lines",
            Some(json!({"side":"white","moves":["e2e4","e7e5","g1f3"]})),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["added"], 3);
        let nf3 = v["path"][2].as_i64().unwrap();
        let e5 = v["path"][1].as_i64().unwrap();

        // Conflict on your move, then replace.
        let (s, v) = call(
            &app,
            Method::POST,
            "/api/repertoire/nodes",
            Some(json!({"side":"white","parent_id":e5,"uci":"f1c4"})),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["conflict"]["existing_san"], "Nf3");
        let (_, v) = call(
            &app,
            Method::POST,
            "/api/repertoire/nodes",
            Some(json!({"side":"white","parent_id":e5,"uci":"g1f3","note":"Develop!"})),
        )
        .await;
        assert_eq!(v["added"], 0);
        assert_eq!(v["path"][0].as_i64(), Some(nf3));

        // Illegal move (localized error).
        let (s, v) = call(
            &app,
            Method::POST,
            "/api/repertoire/nodes?lang=es",
            Some(json!({"side":"white","parent_id":e5,"uci":"e1e5"})),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert!(v["error"].as_str().unwrap().contains("ilegal"), "{v}");

        let (s, v) = call(&app, Method::PATCH, &format!("/api/repertoire/nodes/{nf3}"), Some(json!({"note":"Knight out"}))).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["note"], "Knight out");

        let (_, v) = call(&app, Method::GET, "/api/repertoire/summary", None).await;
        assert_eq!((v["due"].as_u64(), v["lines"].as_u64()), (Some(2), Some(1)));

        let (_, v) = call(&app, Method::GET, "/api/repertoire/drill/next?side=white", None).await;
        let line = v["line"]["nodes"].as_array().unwrap();
        assert_eq!(line.len(), 3);
        let first = line[0]["id"].as_i64().unwrap();
        let (s, v) = call(
            &app,
            Method::POST,
            "/api/repertoire/drill/attempt",
            Some(json!({"node_id":first,"uci":"d2d4"})),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!((v["correct"].as_bool(), v["expected_san"].as_str()), (Some(false), Some("e4")));
        let (s, _) = call(&app, Method::POST, "/api/repertoire/drill/attempt", Some(json!({"node_id":e5,"uci":"e7e5"}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        let (s, v) = call(&app, Method::GET, "/api/repertoire/deviations", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v.as_array().unwrap().len(), 0);

        let (s, v) = call(&app, Method::DELETE, &format!("/api/repertoire/nodes/{e5}"), None).await;
        assert_eq!((s, v["deleted"].as_u64()), (StatusCode::OK, Some(2)));
        let (s, _) = call(&app, Method::DELETE, &format!("/api/repertoire/nodes/{e5}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, v) = call(&app, Method::DELETE, "/api/repertoire?side=white", None).await;
        assert_eq!(v["deleted"], 1);
    }

    #[tokio::test]
    async fn starters_apply() {
        let app = app();
        let (_, v) = call(&app, Method::GET, "/api/repertoire/starters?lang=es", None).await;
        let list = v.as_array().unwrap();
        assert_eq!(list.len(), STARTERS.len());
        assert!(list.iter().all(|s| s["lines"].as_u64().unwrap() >= 5));
        let (s, v) = call(&app, Method::POST, "/api/repertoire/starters/caro-kann-qgd", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["skipped"], 0);
        assert!(v["added"].as_u64().unwrap() > 20);
        // Applying again adds nothing.
        let (_, v) = call(&app, Method::POST, "/api/repertoire/starters/caro-kann-qgd", None).await;
        assert_eq!(v["added"], 0);
        let (s, _) = call(&app, Method::POST, "/api/repertoire/starters/nope", None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, v) = call(&app, Method::GET, "/api/repertoire/summary", None).await;
        assert!(v["black"]["lines"].as_u64().unwrap() >= 8);
    }
}
