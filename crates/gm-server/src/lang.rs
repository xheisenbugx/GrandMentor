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

/// Index into the per-language translation arrays below (`[es, pt, fr, de]`); `None` for English.
fn slot(lang: Lang) -> Option<usize> {
    match lang {
        Lang::En => None,
        Lang::Es => Some(0),
        Lang::Pt => Some(1),
        Lang::Fr => Some(2),
        Lang::De => Some(3),
    }
}

/// Exact-message translations: (English, [Spanish, Portuguese, French, German]).
const EXACT: &[(&str, [&str; 4])] = &[
    (
        "no such API endpoint",
        ["No existe ese endpoint de la API", "Esse endpoint da API não existe", "Ce point d'accès de l'API n'existe pas", "Diesen API-Endpunkt gibt es nicht"],
    ),
    ("the game is already over", ["La partida ya ha terminado", "A partida já terminou", "La partie est déjà terminée", "Die Partie ist bereits beendet"]),
    ("game is over", ["La partida ya ha terminado", "A partida já terminou", "La partie est déjà terminée", "Die Partie ist bereits beendet"]),
    ("missing `fen`", ["Falta el parámetro `fen`", "Falta o parâmetro `fen`", "Le paramètre `fen` est manquant", "Der Parameter `fen` fehlt"]),
    (
        "question must not be empty",
        ["La pregunta no puede estar vacía", "A pergunta não pode estar vazia", "La question ne peut pas être vide", "Die Frage darf nicht leer sein"],
    ),
    (
        "provide `moves`, `pgn` or `game_id`",
        ["Indica `moves`, `pgn` o `game_id`", "Informe `moves`, `pgn` ou `game_id`", "Indique `moves`, `pgn` ou `game_id`", "Gib `moves`, `pgn` oder `game_id` an"],
    ),
    (
        "no game found in PGN",
        [
            "No se encontró ninguna partida en el PGN",
            "Nenhuma partida encontrada no PGN",
            "Aucune partie trouvée dans le PGN",
            "Im PGN wurde keine Partie gefunden",
        ],
    ),
    ("`pgn` must not be empty", ["El `pgn` no puede estar vacío", "O `pgn` não pode estar vazio", "Le `pgn` ne peut pas être vide", "`pgn` darf nicht leer sein"]),
    (
        "no puzzles match that filter",
        [
            "Ningún problema coincide con ese filtro",
            "Nenhum problema corresponde a esse filtro",
            "Aucun problème ne correspond à ce filtre",
            "Keine Aufgabe passt zu diesem Filter",
        ],
    ),
    ("no puzzles available", ["No hay problemas disponibles", "Não há problemas disponíveis", "Aucun problème disponible", "Keine Aufgaben verfügbar"]),
    (
        "name must be 1-40 characters",
        [
            "El nombre debe tener entre 1 y 40 caracteres",
            "O nome deve ter entre 1 e 40 caracteres",
            "Le nom doit contenir entre 1 et 40 caractères",
            "Der Name muss 1 bis 40 Zeichen lang sein",
        ],
    ),
    ("avatar too long", ["El avatar es demasiado largo", "O avatar é longo demais", "L'avatar est trop long", "Der Avatar ist zu lang"]),
    (
        "settings too large",
        ["La configuración es demasiado grande", "As configurações são grandes demais", "Les réglages sont trop volumineux", "Die Einstellungen sind zu groß"],
    ),
    (
        "settings_json must be valid JSON",
        ["settings_json debe ser un JSON válido", "settings_json deve ser um JSON válido", "settings_json doit être un JSON valide", "settings_json muss gültiges JSON sein"],
    ),
    (
        "too many tags (max 32)",
        ["Demasiadas etiquetas (máximo 32)", "Etiquetas demais (máximo de 32)", "Trop d'étiquettes (32 maximum)", "Zu viele Tags (höchstens 32)"],
    ),
    (
        "notes too long (max 20000 characters)",
        [
            "Notas demasiado largas (máximo 20000 caracteres)",
            "Notas longas demais (máximo de 20000 caracteres)",
            "Notes trop longues (20000 caractères maximum)",
            "Notizen zu lang (höchstens 20000 Zeichen)",
        ],
    ),
    (
        "`course_id` and `lesson_id` are required",
        [
            "Se requieren `course_id` y `lesson_id`",
            "`course_id` e `lesson_id` são obrigatórios",
            "`course_id` et `lesson_id` sont obligatoires",
            "`course_id` und `lesson_id` sind erforderlich",
        ],
    ),
    (
        "server is shutting down",
        ["El servidor se está apagando", "O servidor está sendo desligado", "Le serveur est en cours d'arrêt", "Der Server wird heruntergefahren"],
    ),
    ("engine error", ["Error del motor", "Erro do motor", "Erreur du moteur", "Engine-Fehler"]),
    ("analysis error", ["Error en el análisis", "Erro na análise", "Erreur d'analyse", "Analysefehler"]),
    ("analysis was cancelled", ["Se canceló el análisis", "A análise foi cancelada", "L'analyse a été annulée", "Die Analyse wurde abgebrochen"]),
    (
        "engine failed while analysing the game",
        [
            "El motor falló al analizar la partida",
            "O motor falhou ao analisar a partida",
            "Le moteur a échoué pendant l'analyse de la partie",
            "Die Engine ist bei der Analyse der Partie gescheitert",
        ],
    ),
    (
        "analysis returned no moves",
        ["El análisis no devolvió jugadas", "A análise não retornou lances", "L'analyse n'a renvoyé aucun coup", "Die Analyse hat keine Züge geliefert"],
    ),
    ("mentor error", ["Error del mentor", "Erro do mentor", "Erreur du mentor", "Mentor-Fehler"]),
    ("internal error", ["Error interno", "Erro interno", "Erreur interne", "Interner Fehler"]),
    ("internal error (worker panicked)", ["Error interno", "Erro interno", "Erreur interne", "Interner Fehler"]),
    ("FEN too long", ["FEN demasiado largo", "FEN longo demais", "FEN trop longue", "FEN zu lang"]),
    (
        "expected Content-Type: application/json",
        [
            "Se esperaba Content-Type: application/json",
            "Era esperado Content-Type: application/json",
            "Content-Type attendu : application/json",
            "Erwartet wurde Content-Type: application/json",
        ],
    ),
    (
        "a review is already running",
        ["Ya hay una revisión en curso", "Já há uma revisão em andamento", "Une revue est déjà en cours", "Es läuft bereits eine Analyse"],
    ),
    ("`game_id` is required", ["Se requiere `game_id`", "`game_id` é obrigatório", "`game_id` est obligatoire", "`game_id` ist erforderlich"]),
    (
        "null moves are not supported",
        ["No se admiten jugadas nulas", "Lances nulos não são aceitos", "Les coups nuls ne sont pas pris en charge", "Nullzüge werden nicht unterstützt"],
    ),
];

/// Prefix translations: (English prefix, [Spanish, Portuguese, French, German], translate the
/// remainder too).
const PREFIX: &[(&str, [&str; 4], bool)] = &[
    ("invalid FEN: ", ["FEN no válido: ", "FEN inválido: ", "FEN invalide : ", "Ungültige FEN: "], false),
    ("illegal position: ", ["Posición ilegal: ", "Posição ilegal: ", "Position illégale : ", "Illegale Stellung: "], false),
    ("illegal move: ", ["Jugada ilegal: ", "Lance ilegal: ", "Coup illégal : ", "Illegaler Zug: "], false),
    ("illegal move ", ["Jugada ilegal: ", "Lance ilegal: ", "Coup illégal : ", "Illegaler Zug: "], false),
    ("invalid UCI move: ", ["Jugada UCI no válida: ", "Lance UCI inválido: ", "Coup UCI invalide : ", "Ungültiger UCI-Zug: "], false),
    ("invalid UCI move ", ["Jugada UCI no válida: ", "Lance UCI inválido: ", "Coup UCI invalide : ", "Ungültiger UCI-Zug: "], false),
    ("invalid SAN move: ", ["Jugada SAN no válida: ", "Lance SAN inválido: ", "Coup SAN invalide : ", "Ungültiger SAN-Zug: "], false),
    ("unknown bot: ", ["Bot desconocido: ", "Bot desconhecido: ", "Bot inconnu : ", "Unbekannter Bot: "], false),
    ("too many moves ", ["Demasiadas jugadas ", "Lances demais ", "Trop de coups ", "Zu viele Züge "], false),
    (
        "game too long to review ",
        ["Partida demasiado larga para revisarla ", "Partida longa demais para revisar ", "Partie trop longue pour la revue ", "Partie zu lang für die Analyse "],
        false,
    ),
    ("game longer than ", ["Partida más larga de ", "Partida com mais de ", "Partie de plus de ", "Partie länger als "], false),
    (
        "could not import PGN: ",
        ["No se pudo importar el PGN: ", "Não foi possível importar o PGN: ", "Impossible d'importer le PGN : ", "PGN konnte nicht importiert werden: "],
        true,
    ),
    ("internal error: ", ["Error interno: ", "Erro interno: ", "Erreur interne : ", "Interner Fehler: "], false),
    ("invalid query: ", ["Consulta no válida: ", "Consulta inválida: ", "Requête invalide : ", "Ungültige Abfrage: "], false),
    ("invalid path: ", ["Ruta no válida: ", "Caminho inválido: ", "Chemin invalide : ", "Ungültiger Pfad: "], false),
    (
        "invalid request body: ",
        ["Cuerpo de la solicitud no válido: ", "Corpo da requisição inválido: ", "Corps de requête invalide : ", "Ungültiger Anfrageinhalt: "],
        false,
    ),
    ("malformed JSON: ", ["JSON mal formado: ", "JSON malformado: ", "JSON mal formé : ", "Fehlerhaftes JSON: "], false),
    ("invalid result ", ["Resultado no válido: ", "Resultado inválido: ", "Résultat invalide : ", "Ungültiges Ergebnis: "], false),
    ("unsupported variant ", ["Variante no admitida: ", "Variante não suportada: ", "Variante non prise en charge : ", "Nicht unterstützte Variante: "], false),
    ("cannot read move ", ["No se puede leer la jugada ", "Não foi possível ler o lance ", "Impossible de lire le coup ", "Zug nicht lesbar: "], false),
];

/// "<kind> <id> not found" messages: (English kind, [Spanish, Portuguese, French, German] noun
/// phrase as used in [`NOT_FOUND_TPL`]).
const NOT_FOUND: &[(&str, [&str; 4])] = &[
    ("game", ["la partida", "a partida", "la partie", "Partie"]),
    ("puzzle", ["el problema", "o problema", "le problème", "Aufgabe"]),
    ("course", ["el curso", "o curso", "le cours", "Kurs"]),
    ("opening", ["la apertura", "a abertura", "l'ouverture", "Eröffnung"]),
    ("endgame", ["el final", "o final", "la finale", "Endspiel"]),
    ("classic game", ["la partida clásica", "a partida clássica", "la partie classique", "Klassikerpartie"]),
    ("drill", ["el ejercicio", "o exercício", "l'exercice", "Übung"]),
    ("mistake card", ["la tarjeta de error", "o cartão de erro", "la fiche d'erreur", "Fehlerkarte"]),
];

/// "<noun> <id> not found" sentence per language (`{n}` noun phrase, `{id}` the id).
const NOT_FOUND_TPL: [&str; 4] = ["No se encontró {n} {id}", "Não foi possível encontrar {n} {id}", "Impossible de trouver {n} {id}", "{n} {id} wurde nicht gefunden"];

/// "Move {n} illegal: …" (review) and "Move {n}: …" (move-list validation) per language.
const PLY_ILLEGAL: [&str; 4] = ["Jugada {n} ilegal: ", "Lance {n} ilegal: ", "Coup {n} illégal : ", "Zug {n} illegal: "];
const PLY: [&str; 4] = ["Jugada {n}: ", "Lance {n}: ", "Coup {n} : ", "Zug {n}: "];
/// Replacements inside untranslated remainders: "(max " and " plies".
const MAX_WORD: [&str; 4] = ["(máximo ", "(máximo de ", "(maximum ", "(höchstens "];
const PLIES_WORD: [&str; 4] = [" medias jugadas", " meios-lances", " demi-coups", " Halbzüge"];

/// Translate a server error message into `lang`. Unknown messages are returned unchanged.
pub fn translate_error(msg: &str, lang: Lang) -> String {
    match slot(lang) {
        None => msg.to_string(),
        Some(i) => translate(msg, i, 0),
    }
}

fn capitalize(s: String) -> String {
    gm_content::words::capitalize(&s)
}

/// German nouns stay capitalized; elsewhere the inner message continues the sentence.
fn continue_sentence(s: &str, i: usize) -> String {
    if i == 3 {
        s.to_string()
    } else {
        lower_first(s)
    }
}

fn translate(msg: &str, i: usize, depth: u8) -> String {
    if depth > 4 {
        return msg.to_string();
    }
    if let Some((_, tr)) = EXACT.iter().find(|(en, _)| *en == msg) {
        return tr[i].to_string();
    }
    // "ply 3: <inner>" (move-list validation) and "illegal move at ply 3: <inner>" (review).
    if let Some(rest) = msg.strip_prefix("illegal move at ply ") {
        if let Some((n, inner)) = rest.split_once(": ") {
            let head = PLY_ILLEGAL[i].replace("{n}", n);
            return format!("{head}{}", continue_sentence(&translate(inner, i, depth + 1), i));
        }
    }
    if let Some(rest) = msg.strip_prefix("ply ") {
        if let Some((n, inner)) = rest.split_once(": ") {
            if n.chars().all(|c| c.is_ascii_digit()) {
                let head = PLY[i].replace("{n}", n);
                return format!("{head}{}", continue_sentence(&translate(inner, i, depth + 1), i));
            }
        }
    }
    if let Some((en, tr, recurse)) = PREFIX.iter().find(|(en, _, _)| msg.starts_with(en)) {
        let rest = &msg[en.len()..];
        let rest = if *recurse { translate(rest, i, depth + 1) } else { rest.to_string() };
        let rest = rest.replace("(max ", MAX_WORD[i]).replace(" plies", PLIES_WORD[i]);
        return format!("{}{rest}", tr[i]);
    }
    if let Some(body) = msg.strip_suffix(" not found") {
        for (kind, nouns) in NOT_FOUND {
            if let Some(id) = body.strip_prefix(kind).and_then(|r| r.strip_prefix(' ')) {
                return capitalize(NOT_FOUND_TPL[i].replace("{n}", nouns[i]).replace("{id}", id));
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
        let uri: Uri = "/api/bots?lang=pt-BR".parse().unwrap();
        assert_eq!(lang_of(&uri, &h), Lang::Pt);
        let uri: Uri = "/api/bots".parse().unwrap();
        for (header, lang) in [("fr-CA,fr;q=0.9", Lang::Fr), ("de-AT", Lang::De), ("pt-PT,en;q=0.5", Lang::Pt), ("it-IT,de;q=0.4", Lang::De)] {
            h.insert(header::ACCEPT_LANGUAGE, HeaderValue::from_static(header));
            assert_eq!(lang_of(&uri, &h), lang, "{header}");
        }
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

    #[test]
    fn errors_translate_pt_fr_de() {
        let pt = |m: &str| translate_error(m, Lang::Pt);
        assert_eq!(pt("game 42 not found"), "Não foi possível encontrar a partida 42");
        assert_eq!(pt("ply 2: illegal move: e2e5"), "Lance 2: lance ilegal: e2e5");
        assert_eq!(pt("too many moves (max 1200)"), "Lances demais (máximo de 1200)");
        assert_eq!(pt("game too long to review (900 plies, max 600)"), "Partida longa demais para revisar (900 meios-lances, max 600)");
        assert_eq!(pt("no puzzles available"), "Não há problemas disponíveis");

        let fr = |m: &str| translate_error(m, Lang::Fr);
        assert_eq!(fr("opening \"x\" not found"), "Impossible de trouver l'ouverture \"x\"");
        assert_eq!(fr("illegal move at ply 1: illegal move: e2e5"), "Coup 1 illégal : coup illégal : e2e5");
        assert_eq!(fr("could not import PGN: illegal move 3.Ke9"), "Impossible d'importer le PGN : Coup illégal : 3.Ke9");
        assert_eq!(fr("invalid FEN: bad board"), "FEN invalide : bad board");
        assert_eq!(fr("the game is already over"), "La partie est déjà terminée");

        let de = |m: &str| translate_error(m, Lang::De);
        assert_eq!(de("game 42 not found"), "Partie 42 wurde nicht gefunden");
        assert_eq!(de("puzzle abc not found"), "Aufgabe abc wurde nicht gefunden");
        assert_eq!(de("ply 2: illegal move: e2e5"), "Zug 2: Illegaler Zug: e2e5");
        assert_eq!(de("too many moves (max 1200)"), "Zu viele Züge (höchstens 1200)");
        assert_eq!(de("question must not be empty"), "Die Frage darf nicht leer sein");
        assert_eq!(de("something new"), "something new");
    }

    /// Every table entry has a non-empty translation in every language, and every message
    /// actually changes when translated.
    #[test]
    fn tables_complete_in_every_language() {
        for lang in Lang::ALL {
            let Some(i) = slot(lang) else { continue };
            for (en, tr) in EXACT {
                assert!(!tr[i].trim().is_empty(), "{lang}: {en}");
                assert_ne!(translate_error(en, lang), *en, "{lang}: {en}");
            }
            for (en, tr, _) in PREFIX {
                assert!(!tr[i].trim().is_empty(), "{lang}: {en}");
                let msg = format!("{en}x");
                assert_ne!(translate_error(&msg, lang), msg, "{lang}: {en}");
            }
            for (kind, nouns) in NOT_FOUND {
                assert!(!nouns[i].trim().is_empty(), "{lang}: {kind}");
                let msg = format!("{kind} 7 not found");
                let out = translate_error(&msg, lang);
                assert!(out.contains('7') && out != msg, "{lang}: {out}");
            }
        }
    }
}
