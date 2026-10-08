//! Anthropic Messages API client for the mentor chat (raw HTTP via reqwest).

use std::time::Duration;

use serde_json::{json, Value};
use shakmaty::{Chess, File, Position, Rank, Square};

use gm_content::Lang;

use crate::{coach, describe, tactics, ChatRequest};

pub(crate) const API_URL: &str = "https://api.anthropic.com/v1/messages";
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TOKENS: u32 = 2048;
const MAX_QUESTION_CHARS: usize = 2000;
const MAX_TURN_CHARS: usize = 2000;
const MAX_HISTORY_TURNS: usize = 12;
const MAX_MOVES: usize = 400;
const MAX_ENGINE_LINES: usize = 5;
const MAX_ERROR_BODY: usize = 300;

pub(crate) const SYSTEM_PROMPT: &str = "You are Mentor Mira, a warm, patient chess coach inside the GrandMentor learning app. \
Your students are mostly beginners and improving club players.

How to answer:
- Ground every claim in the position data you are given: the FEN, the board diagram, the move history, the list of legal moves, the engine lines and the coach notes. Treat the engine lines as the source of truth for evaluation and best moves.
- Never invent moves. Only mention moves for the side to move that appear in the legal-move list, and only mention longer variations that appear in the engine lines. If you are unsure whether a move is legal or good, say so rather than guessing.
- Name squares and pieces concretely (\"your knight on f3\", \"the weak pawn on d5\") and write moves in SAN, wrapped in **bold**, e.g. **Nf3**.
- Explain the *why*: the idea, plan, or tactic (fork, pin, skewer, hanging piece, back-rank weakness, king safety, center control, development, passed pawns, open files). Turn engine numbers into plain words (+1.0 is roughly a pawn's worth).
- Be encouraging and concise: usually 2 to 5 short sentences or a few bullet points, under 150 words. No headings. Avoid jargon, or explain it in a few words when you use it.
- Stay on chess. If the question is not about chess, gently steer back to the game.";

/// System prompt with the answer-language instruction for `lang`.
pub(crate) fn system_prompt(lang: Lang) -> String {
    let language = match lang {
        Lang::En => "- Answer in English.",
        Lang::Es => "- Answer in Spanish (español), the student's language, even if parts of the context are in English. \
Address the student informally with \"tú\" in a warm, encouraging tone. Use standard Spanish chess terms: rey, dama, torre, alfil, \
caballo, peón, jaque, jaque mate, enroque, clavada, ataque doble, enfilada, pieza colgada, peón pasado, columna abierta, \
apertura, medio juego, final. Keep moves in English SAN exactly as given (e.g. **Nf3**, **O-O**); never translate move notation.",
        Lang::Pt => "- Answer in Brazilian Portuguese (português do Brasil), the student's language, even if parts of the context are in English. \
Address the student informally with \"você\" in a warm, encouraging tone. Use standard Brazilian chess terms: rei, dama, torre, bispo, \
cavalo, peão, lance, xeque, xeque-mate, roque, cravada, garfo, espeto, peça pendurada, peão passado, coluna aberta, \
abertura, meio-jogo, final; the sides are \"as brancas\" and \"as pretas\". Keep moves in English SAN exactly as given \
(e.g. **Nf3**, **O-O**); never translate move notation.",
        Lang::Fr => "- Answer in French (français), the student's language, even if parts of the context are in English. \
Address the student informally with \"tu\" in a warm, encouraging tone. Use standard French chess terms: roi, dame, tour, fou, \
cavalier, pion, coup, échec, échec et mat, roque, clouage, fourchette, enfilade, pièce en prise, pion passé, colonne ouverte, \
ouverture, milieu de partie, finale; the sides are \"les Blancs\" and \"les Noirs\". Keep moves in English SAN exactly as given \
(e.g. **Nf3**, **O-O**, not Cf3); never translate move notation.",
        Lang::De => "- Answer in German (Deutsch), the student's language, even if parts of the context are in English. \
Address the student informally with \"du\" in a warm, encouraging tone. Use standard German chess terms: König, Dame, Turm, Läufer, \
Springer, Bauer, Zug, Schach, Schachmatt, Rochade, Fesselung, Gabel, Spieß, ungedeckte Figur, Freibauer, offene Linie, \
Eröffnung, Mittelspiel, Endspiel; the sides are \"Weiß\" and \"Schwarz\". Keep moves in English SAN exactly as given \
(e.g. **Nf3**, **O-O**, not Sf3); never translate move notation.",
    };
    format!("{SYSTEM_PROMPT}\n{language}")
}

/// True for models that accept `output_config.effort`.
fn supports_effort(model: &str) -> bool {
    ["claude-opus-5", "claude-fable-5", "claude-mythos-5", "claude-sonnet-5", "claude-haiku-5-5", "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6"]
        .iter()
        .any(|p| model.starts_with(p))
}

/// True for models that accept the server-side refusal fallback (`fallbacks: "default"`).
fn supports_fallbacks(model: &str) -> bool {
    matches!(model, "claude-opus-5-5" | "claude-opus-5" | "claude-fable-5-1" | "claude-sonnet-5-5")
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

/// ASCII diagram, White at the bottom; uppercase = White.
pub(crate) fn ascii_board(pos: &Chess) -> String {
    let b = pos.board();
    let mut out = String::with_capacity(200);
    for r in (0..8u32).rev() {
        out.push(char::from(b'1' + r as u8));
        out.push_str(" |");
        for f in 0..8u32 {
            let sq = Square::from_coords(File::new(f), Rank::new(r));
            out.push(' ');
            out.push(b.piece_at(sq).map(|p| p.char()).unwrap_or('.'));
        }
        out.push('\n');
    }
    out.push_str("  +----------------\n    a b c d e f g h");
    out
}

fn numbered_history(moves: &[String]) -> String {
    let start = moves.len().saturating_sub(MAX_MOVES);
    let mut s = String::new();
    if start > 0 {
        s.push_str("… ");
    }
    for (i, m) in moves.iter().enumerate().skip(start) {
        if i % 2 == 0 {
            s.push_str(&format!("{}. ", i / 2 + 1));
        }
        s.push_str(&truncate_chars(m, 10));
        s.push(' ');
    }
    s.trim_end().to_string()
}

/// The final user message: position context + question.
pub(crate) fn build_context(req: &ChatRequest) -> String {
    let mut s = String::new();
    s.push_str("<position>\n");
    match gm_engine::parse_fen(&req.fen) {
        Ok(pos) => {
            s.push_str(&format!("FEN: {}\n", gm_engine::to_fen(&pos)));
            s.push_str(&format!("Side to move: {}\n", coach::side_name(pos.turn())));
            if pos.is_checkmate() {
                s.push_str("Status: checkmate (game over)\n");
            } else if pos.is_stalemate() {
                s.push_str("Status: stalemate (draw)\n");
            } else if pos.is_check() {
                s.push_str("Status: the side to move is in check\n");
            }
            s.push_str("Board (uppercase = White, lowercase = Black):\n");
            s.push_str(&ascii_board(&pos));
            s.push('\n');
            let legal: Vec<String> = pos.legal_moves().iter().map(|m| tactics::san(&pos, m)).collect();
            if !legal.is_empty() {
                s.push_str(&format!("Legal moves for {}: {}\n", coach::side_name(pos.turn()), legal.join(" ")));
            }
            let notes = describe::describe(&pos, Lang::En);
            if !notes.is_empty() {
                s.push_str("Coach notes (rule-based, reliable):\n");
                for n in notes {
                    s.push_str(&format!("- {}\n", n.text));
                }
            }
        }
        Err(_) => {
            s.push_str(&format!("FEN (could not be parsed): {}\n", truncate_chars(&req.fen, 120)));
        }
    }
    if req.moves_san.is_empty() {
        s.push_str("Move history: (none)\n");
    } else {
        s.push_str(&format!("Move history (SAN): {}\n", numbered_history(&req.moves_san)));
    }
    let lines: Vec<String> = req.engine_lines.iter().take(MAX_ENGINE_LINES).map(|l| truncate_chars(l.trim(), 300)).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        s.push_str("Engine lines: (not available — rely on the coach notes and avoid concrete evaluations)\n");
    } else {
        s.push_str("Engine lines (best first; scores from White's point of view, in pawns; M = forced mate):\n");
        for l in lines {
            s.push_str(&format!("- {l}\n"));
        }
    }
    s.push_str("</position>\n\n");
    s.push_str("Student's question: ");
    s.push_str(&truncate_chars(req.question.trim(), MAX_QUESTION_CHARS));
    s
}

/// Messages array: prior turns (alternating, starting with the user) + the final context message.
pub(crate) fn build_messages(req: &ChatRequest) -> Vec<Value> {
    let mut msgs: Vec<(&'static str, String)> = Vec::new();
    let start = req.history.len().saturating_sub(MAX_HISTORY_TURNS);
    for t in &req.history[start..] {
        let role = match t.role.trim().to_ascii_lowercase().as_str() {
            "user" => "user",
            "mentor" | "assistant" | "coach" => "assistant",
            _ => continue,
        };
        let text = truncate_chars(t.text.trim(), MAX_TURN_CHARS);
        if text.is_empty() {
            continue;
        }
        match msgs.last_mut() {
            Some((r, existing)) if *r == role => {
                existing.push_str("\n\n");
                existing.push_str(&text);
            }
            _ => msgs.push((role, text)),
        }
    }
    while msgs.first().map(|(r, _)| *r == "assistant").unwrap_or(false) {
        msgs.remove(0);
    }
    let ctx = build_context(req);
    match msgs.last_mut() {
        Some((r, existing)) if *r == "user" => {
            existing.push_str("\n\n");
            existing.push_str(&ctx);
        }
        _ => msgs.push(("user", ctx)),
    }
    msgs.into_iter().map(|(role, text)| json!({"role": role, "content": text})).collect()
}

pub(crate) fn build_body(model: &str, req: &ChatRequest, extras: bool, lang: Lang) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": system_prompt(lang),
        "messages": build_messages(req),
    });
    if extras {
        if supports_effort(model) {
            body["output_config"] = json!({"effort": "low"});
        }
        if supports_fallbacks(model) {
            body["fallbacks"] = json!("default");
        }
    }
    body
}

#[derive(Debug)]
pub(crate) enum LlmError {
    /// 400-class error that might be caused by optional parameters; worth one retry without them.
    BadRequest(String),
    Other(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::BadRequest(s) | LlmError::Other(s) => f.write_str(s),
        }
    }
}

/// Extract the answer text from a Messages API response.
pub(crate) fn parse_response(v: &Value) -> Result<String, LlmError> {
    let stop = v.get("stop_reason").and_then(Value::as_str).unwrap_or("");
    if stop == "refusal" {
        return Err(LlmError::Other("model declined the request".into()));
    }
    let text: String = v
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(LlmError::Other(format!("empty response (stop_reason={stop})")));
    }
    Ok(text)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn call(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    model: &str,
    req: &ChatRequest,
    extras: bool,
    lang: Lang,
) -> Result<String, LlmError> {
    let body = build_body(model, req, extras, lang);
    let mut rb = client
        .post(url)
        .timeout(REQUEST_TIMEOUT)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json");
    if extras && supports_fallbacks(model) {
        rb = rb.header("anthropic-beta", "server-side-fallback-2026-07-01");
    }
    let resp = rb.json(&body).send().await.map_err(|e| LlmError::Other(format!("request failed: {}", e.without_url())))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| LlmError::Other(format!("reading body failed: {}", e.without_url())))?;
    if !status.is_success() {
        let snippet = truncate_chars(&String::from_utf8_lossy(&bytes), MAX_ERROR_BODY);
        let msg = format!("HTTP {status}: {snippet}");
        return Err(if status.as_u16() == 400 { LlmError::BadRequest(msg) } else { LlmError::Other(msg) });
    }
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| LlmError::Other(format!("invalid JSON: {e}")))?;
    parse_response(&v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatTurn;

    fn req() -> ChatRequest {
        ChatRequest {
            question: "What's the plan?".into(),
            fen: "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2".into(),
            moves_san: vec!["e4".into(), "e5".into(), "Nf3".into()],
            engine_lines: vec!["+0.30: Nc6 Bb5 a6".into()],
            history: vec![
                ChatTurn { role: "mentor".into(), text: "Welcome!".into() },
                ChatTurn { role: "user".into(), text: "hi".into() },
                ChatTurn { role: "mentor".into(), text: "Hello!".into() },
            ],
        }
    }

    #[test]
    fn body_shape() {
        let body = build_body("claude-opus-5-5", &req(), true, Lang::En);
        assert!(body["system"].as_str().unwrap_or("").ends_with("Answer in English."));
        let es = build_body("claude-opus-5-5", &req(), true, Lang::Es);
        assert!(es["system"].as_str().unwrap_or("").contains("Answer in Spanish"));
        for (lang, needle, term) in [
            (Lang::Pt, "Answer in Brazilian Portuguese", "xeque-mate"),
            (Lang::Fr, "Answer in French", "échec et mat"),
            (Lang::De, "Answer in German", "Schachmatt"),
        ] {
            let b = build_body("claude-opus-5-5", &req(), true, lang);
            let system = b["system"].as_str().unwrap_or("");
            assert!(system.contains(needle) && system.contains(term) && system.contains("English SAN"), "{lang}: {system}");
            assert!(system.starts_with(SYSTEM_PROMPT));
        }
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["output_config"]["effort"], "low");
        assert_eq!(body["fallbacks"], "default");
        let msgs = body["messages"].as_array().expect("messages");
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs.last().map(|m| m["role"].clone()), Some(json!("user")));
        for w in msgs.windows(2) {
            assert_ne!(w[0]["role"], w[1]["role"]);
        }
        let last = msgs.last().and_then(|m| m["content"].as_str()).unwrap_or("");
        assert!(last.contains("FEN: rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2"));
        assert!(last.contains("1. e4 e5 2. Nf3"));
        assert!(last.contains("+0.30: Nc6 Bb5 a6"));
        assert!(last.contains("Legal moves for Black:"));
        assert!(last.contains("8 | r n b q k b n r"));
        let plain = build_body("claude-haiku-4-5", &req(), true, Lang::En);
        assert!(plain.get("output_config").is_none());
        assert!(plain.get("fallbacks").is_none());
    }

    #[test]
    fn response_parsing() {
        let ok = json!({"content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "Play **Nc6**."}], "stop_reason": "end_turn"});
        assert_eq!(parse_response(&ok).ok().as_deref(), Some("Play **Nc6**."));
        let refusal = json!({"content": [], "stop_reason": "refusal"});
        assert!(parse_response(&refusal).is_err());
        assert!(parse_response(&json!({})).is_err());
    }
}
