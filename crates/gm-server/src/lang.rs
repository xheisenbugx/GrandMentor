//! Request language: `?lang=` first, then `Accept-Language`, then English (see `docs/I18N.md`).
//!
//! * [`ReqLang`] — extractor handing the negotiated [`Lang`] to handlers that return text.
//! * [`localize_errors`] — middleware translating `{ "error": ... }` bodies of 4xx/5xx
//!   responses (including extractor rejections and the API 404) into the request language.

use std::convert::Infallible;

use axum::body::Body;
use axum::extract::{FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, Uri};
use axum::middleware::Next;
use axum::response::Response;
use gm_content::Lang;

/// Largest error body we rewrite (error bodies are tiny; anything bigger passes through).
const MAX_ERROR_BODY: usize = 64 * 1024;

/// The language negotiated for this request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReqLang(pub Lang);

/// `lang` query parameter, if present.
fn query_lang(uri: &Uri) -> Option<&str> {
    uri.query()?
        .split('&')
        .take(64)
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == "lang")
        .map(|(_, v)| v)
}

/// Negotiate the language from a request's URI and headers.
pub fn lang_of(uri: &Uri, headers: &HeaderMap) -> Lang {
    let accept = headers.get(header::ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok());
    Lang::negotiate(query_lang(uri), accept)
}

#[axum::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for ReqLang {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(ReqLang(lang_of(&parts.uri, &parts.headers)))
    }
}

/// Rewrites the `error` message of failed API responses into the request language.
pub async fn localize_errors(req: Request, next: Next) -> Response {
    let lang = lang_of(req.uri(), req.headers());
    let resp = next.run(req).await;
    let status = resp.status();
    if lang == Lang::En || !(status.is_client_error() || status.is_server_error()) {
        return resp;
    }
    let is_json = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return resp;
    }
    let (mut parts, body) = resp.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_ERROR_BODY).await {
        Ok(b) => b,
        Err(_) => {
            parts.headers.remove(header::CONTENT_LENGTH);
            let msg = serde_json::json!({ "error": translate_error("internal error", lang) }).to_string();
            return Response::from_parts(parts, Body::from(msg));
        }
    };
    let new_body = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(mut v) => match v.get("error").and_then(|e| e.as_str()).map(|m| translate_error(m, lang)) {
            Some(translated) => {
                v["error"] = serde_json::Value::String(translated);
                v.to_string().into_bytes()
            }
            None => bytes.to_vec(),
        },
        Err(_) => bytes.to_vec(),
    };
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(new_body))
}

/// Exact-message translations: (English, Spanish).
const EXACT: &[(&str, &str)] = &[
    ("no such API endpoint", "No existe ese endpoint de la API"),
    ("the game is already over", "La partida ya ha terminado"),
    ("game is over", "La partida ya ha terminado"),
    ("missing `fen`", "Falta el parámetro `fen`"),
    ("question must not be empty", "La pregunta no puede estar vacía"),
    ("provide `moves`, `pgn` or `game_id`", "Indica `moves`, `pgn` o `game_id`"),
    ("no game found in PGN", "No se encontró ninguna partida en el PGN"),
    ("`pgn` must not be empty", "El `pgn` no puede estar vacío"),
    ("no puzzles match that filter", "Ningún problema coincide con ese filtro"),
    ("no puzzles available", "No hay problemas disponibles"),
    ("name must be 1-40 characters", "El nombre debe tener entre 1 y 40 caracteres"),
    ("avatar too long", "El avatar es demasiado largo"),
    ("settings too large", "La configuración es demasiado grande"),
    ("settings_json must be valid JSON", "settings_json debe ser un JSON válido"),
    ("too many tags (max 32)", "Demasiadas etiquetas (máximo 32)"),
    ("notes too long (max 20000 characters)", "Notas demasiado largas (máximo 20000 caracteres)"),
    ("`course_id` and `lesson_id` are required", "Se requieren `course_id` y `lesson_id`"),
    ("server is shutting down", "El servidor se está apagando"),
    ("engine error", "Error del motor"),
    ("analysis error", "Error en el análisis"),
    ("analysis was cancelled", "Se canceló el análisis"),
    ("engine failed while analysing the game", "El motor falló al analizar la partida"),
    ("analysis returned no moves", "El análisis no devolvió jugadas"),
    ("mentor error", "Error del mentor"),
    ("internal error", "Error interno"),
    ("internal error (worker panicked)", "Error interno"),
    ("FEN too long", "FEN demasiado largo"),
    ("expected Content-Type: application/json", "Se esperaba Content-Type: application/json"),
    ("null moves are not supported", "No se admiten jugadas nulas"),
];

/// Prefix translations: (English prefix, Spanish prefix, translate the remainder too).
const PREFIX: &[(&str, &str, bool)] = &[
    ("invalid FEN: ", "FEN no válido: ", false),
    ("illegal position: ", "Posición ilegal: ", false),
    ("illegal move: ", "Jugada ilegal: ", false),
    ("illegal move ", "Jugada ilegal: ", false),
    ("invalid UCI move: ", "Jugada UCI no válida: ", false),
    ("invalid UCI move ", "Jugada UCI no válida: ", false),
    ("invalid SAN move: ", "Jugada SAN no válida: ", false),
    ("unknown bot: ", "Bot desconocido: ", false),
    ("too many moves ", "Demasiadas jugadas ", false),
    ("game too long to review ", "Partida demasiado larga para revisarla ", false),
    ("game longer than ", "Partida más larga de ", false),
    ("could not import PGN: ", "No se pudo importar el PGN: ", true),
    ("internal error: ", "Error interno: ", false),
    ("invalid query: ", "Consulta no válida: ", false),
    ("invalid path: ", "Ruta no válida: ", false),
    ("invalid request body: ", "Cuerpo de la solicitud no válido: ", false),
    ("malformed JSON: ", "JSON mal formado: ", false),
    ("invalid result ", "Resultado no válido: ", false),
    ("unsupported variant ", "Variante no admitida: ", false),
    ("cannot read move ", "No se puede leer la jugada ", false),
];

/// "<kind> <id> not found" messages: (English kind, Spanish noun phrase).
const NOT_FOUND: &[(&str, &str)] = &[
    ("game", "la partida"),
    ("puzzle", "el problema"),
    ("course", "el curso"),
    ("opening", "la apertura"),
    ("endgame", "el final"),
];

/// Translate a server error message into `lang`. Unknown messages are returned unchanged.
pub fn translate_error(msg: &str, lang: Lang) -> String {
    match lang {
        Lang::En => msg.to_string(),
        Lang::Es => translate_es(msg, 0),
    }
}

fn capitalize(s: String) -> String {
    gm_content::words::capitalize(&s)
}

fn translate_es(msg: &str, depth: u8) -> String {
    if depth > 4 {
        return msg.to_string();
    }
    if let Some((_, es)) = EXACT.iter().find(|(en, _)| *en == msg) {
        return (*es).to_string();
    }
    // "ply 3: <inner>" (move-list validation) and "illegal move at ply 3: <inner>" (review).
    if let Some(rest) = msg.strip_prefix("illegal move at ply ") {
        if let Some((n, inner)) = rest.split_once(": ") {
            return format!("Jugada {n} ilegal: {}", lower_first(&translate_es(inner, depth + 1)));
        }
    }
    if let Some(rest) = msg.strip_prefix("ply ") {
        if let Some((n, inner)) = rest.split_once(": ") {
            if n.chars().all(|c| c.is_ascii_digit()) {
                return format!("Jugada {n}: {}", lower_first(&translate_es(inner, depth + 1)));
            }
        }
    }
    if let Some((en, es, recurse)) = PREFIX.iter().find(|(en, _, _)| msg.starts_with(en)) {
        let rest = &msg[en.len()..];
        let rest = if *recurse { translate_es(rest, depth + 1) } else { rest.to_string() };
        let rest = rest.replace("(max ", "(máximo ").replace(" plies", " medias jugadas");
        return format!("{es}{rest}");
    }
    if let Some(body) = msg.strip_suffix(" not found") {
        for (kind, noun) in NOT_FOUND {
            if let Some(id) = body.strip_prefix(kind).and_then(|r| r.strip_prefix(' ')) {
                return capitalize(format!("no se encontró {noun} {id}"));
            }
        }
    }
    msg.to_string()
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn negotiation() {
        let mut h = HeaderMap::new();
        let uri: Uri = "/api/bots".parse().unwrap();
        assert_eq!(lang_of(&uri, &h), Lang::En);
        h.insert(header::ACCEPT_LANGUAGE, HeaderValue::from_static("es-MX,es;q=0.9,en;q=0.8"));
        assert_eq!(lang_of(&uri, &h), Lang::Es);
        let uri: Uri = "/api/bots?x=1&lang=en".parse().unwrap();
        assert_eq!(lang_of(&uri, &h), Lang::En, "query wins");
        let uri: Uri = "/api/bots?lang=zz".parse().unwrap();
        assert_eq!(lang_of(&uri, &h), Lang::Es, "unsupported query falls back to the header");
    }

    #[test]
    fn errors_translate() {
        let es = |m: &str| translate_error(m, Lang::Es);
        assert_eq!(es("game 42 not found"), "No se encontró la partida 42");
        assert_eq!(es("opening \"x\" not found"), "No se encontró la apertura \"x\"");
        assert_eq!(es("invalid FEN: bad board"), "FEN no válido: bad board");
        assert_eq!(es("ply 2: illegal move: e2e5"), "Jugada 2: jugada ilegal: e2e5");
        assert_eq!(es("illegal move at ply 1: illegal move: e2e5"), "Jugada 1 ilegal: jugada ilegal: e2e5");
        assert_eq!(es("could not import PGN: illegal move 3.Ke9"), "No se pudo importar el PGN: Jugada ilegal: 3.Ke9");
        assert_eq!(es("no game found in PGN"), "No se encontró ninguna partida en el PGN");
        assert_eq!(es("too many moves (max 1200)"), "Demasiadas jugadas (máximo 1200)");
        assert_eq!(es("something new"), "something new");
        assert_eq!(translate_error("game 1 not found", Lang::En), "game 1 not found");
    }
}
