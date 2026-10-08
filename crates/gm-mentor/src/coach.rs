//! Rule-based chat answers (used when no LLM key is configured or the LLM call fails).
//!
//! Questions are routed by keywords in every supported language (so "¿qué debo hacer?" and
//! "what should I do?" both ask for the best move); the answer is written in the requested
//! [`Lang`].

use gm_content::words::{capitalize, fill, kv, side_name as side_word, PieceRef};
use gm_content::Lang;
use shakmaty::{Chess, Color, Position};

use crate::describe::{self, Phase};
use crate::explain::plain_move_comment;
use crate::phrase::Picker;
use crate::tactics;
use crate::ChatRequest;

/// One parsed engine line like "+0.45: Nf3 Nc6 Bb5".
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EngineLine {
    /// White-POV evaluation in pawns (mates mapped to +-1000 + distance info).
    pub pawns: Option<f32>,
    /// Mate in N (positive = White mates).
    pub mate: Option<i32>,
    pub score_text: String,
    pub moves: Vec<String>,
}

pub(crate) fn parse_engine_line(s: &str) -> Option<EngineLine> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (score_text, rest) = match s.split_once(':') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => ("", s),
    };
    let moves: Vec<String> = rest
        .split_whitespace()
        .filter(|t| !t.ends_with('.') && !t.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .map(|t| t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.').to_string())
        .filter(|t| !t.is_empty())
        .take(24)
        .collect();
    let st = score_text.replace(' ', "");
    let (mut pawns, mut mate) = (None, None);
    let lower = st.to_ascii_lowercase();
    if let Some(idx) = lower.find(['m', '#']) {
        let sign = if lower.starts_with('-') { -1 } else { 1 };
        let digits: String = lower[idx + 1..].chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<i32>() {
            mate = Some(sign * n.max(1));
        }
    } else if let Ok(v) = st.trim_start_matches('+').parse::<f32>() {
        if v.is_finite() {
            pawns = Some(v.clamp(-99.0, 99.0));
        }
    }
    Some(EngineLine { pawns, mate, score_text: score_text.to_string(), moves })
}

fn t<'a>(lang: Lang, en: &'a str, es: &'a str) -> &'a str {
    match lang {
        Lang::En => en,
        Lang::Es => es,
    }
}

fn tl<'a>(lang: Lang, en: &'a [&'a str], es: &'a [&'a str]) -> &'a [&'a str] {
    match lang {
        Lang::En => en,
        Lang::Es => es,
    }
}

/// Plain-language verdict for a White-POV engine line.
pub(crate) fn eval_words(line: &EngineLine, lang: Lang) -> Option<String> {
    if let Some(m) = line.mate {
        let side = capitalize(side_word(if m > 0 { Color::White } else { Color::Black }, lang));
        let tpl = t(lang, "{S} has a forced checkmate in {n}.", "{S} tienen un mate forzado en {n}.");
        return Some(fill(tpl, &kv(&[("S", &side), ("n", &m.abs().to_string())])));
    }
    let v = line.pawns?;
    let (side, a) = if v >= 0.0 { (Color::White, v) } else { (Color::Black, -v) };
    let tpl = if a < 0.3 {
        t(lang, "The position is roughly equal ({e}).", "La posición está más o menos igualada ({e}).")
    } else if a < 1.0 {
        t(lang, "{S} is slightly better ({e}).", "{S} están algo mejor ({e}).")
    } else if a < 2.5 {
        t(lang, "{S} is clearly better ({e}).", "{S} están claramente mejor ({e}).")
    } else {
        t(lang, "{S} is winning ({e}).", "{S} están ganando ({e}).")
    };
    Some(fill(tpl, &kv(&[("S", &capitalize(side_word(side, lang))), ("e", &line.score_text)])))
}

/// Try to reconstruct the position before the last move by replaying `moves_san` from the
/// standard start. Only works for games that began from the initial position.
fn previous_position(req: &ChatRequest, current: &Chess) -> Option<(Chess, shakmaty::Move)> {
    if req.moves_san.is_empty() || req.moves_san.len() > 600 {
        return None;
    }
    let mut pos = Chess::default();
    let mut prev = None;
    for s in &req.moves_san {
        let m = tactics::parse_san(&pos, s)?;
        prev = Some((pos.clone(), m.clone()));
        pos.play_unchecked(&m);
    }
    if pos.board() == current.board() && pos.turn() == current.turn() {
        prev
    } else {
        None
    }
}

/// Spanish piece letters (C=caballo, A=alfil, D=dama, T=torre, R=rey) to English SAN.
fn spanish_san(tok: &str) -> Option<String> {
    let mut chars = tok.chars();
    let first = match chars.next()? {
        'C' => 'N',
        'A' => 'B',
        'D' => 'Q',
        'T' => 'R',
        'R' => 'K',
        _ => return None,
    };
    Some(std::iter::once(first).chain(chars).collect())
}

fn find_move_in_question(pos: &Chess, q: &str, lang: Lang) -> Option<shakmaty::Move> {
    for raw in q.split(|c: char| c.is_whitespace() || matches!(c, ',' | '?' | '¿' | '¡' | '"' | '\'' | '(' | ')' | ';' | '«' | '»')) {
        let tok = raw.trim_matches(|c: char| c == '.' || c == '!' || c == ':');
        let tok = tok.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        if tok.len() < 2 || tok.len() > 8 {
            continue;
        }
        // Avoid treating plain words as moves: SAN starts with a piece letter, a file, or O.
        let first = tok.chars().next().unwrap_or(' ');
        let spanish_letter = lang == Lang::Es && matches!(first, 'C' | 'A' | 'D' | 'T');
        if !(matches!(first, 'K' | 'Q' | 'R' | 'B' | 'N' | 'O' | '0') || ('a'..='h').contains(&first) || spanish_letter) {
            continue;
        }
        if tok.chars().all(|c| c.is_ascii_alphabetic()) && !tok.starts_with('O') && tok.len() > 3 {
            continue; // a plain word like "bishop" / "dama"
        }
        if let Some(m) = tactics::parse_any_move(pos, tok) {
            return Some(m);
        }
        if lang == Lang::Es {
            if let Some(m) = spanish_san(tok).and_then(|s| tactics::parse_san(pos, &s)) {
                return Some(m);
            }
        }
    }
    None
}

/// Lowercase, strip accents and Spanish inverted punctuation, so keyword routing works for
/// "¿Qué debo hacer?" as well as "que debo hacer".
fn normalize(q: &str) -> String {
    q.to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '¿' | '¡'))
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

fn has_any(q: &str, words: &[&str]) -> bool {
    words.iter().any(|w| q.contains(w))
}

/// Keyword lists; each intent lists English and Spanish (normalized, accent-free) cues.
const LAST_MOVE: &[&str] = &[
    "last move", "my move", "that move", "previous move", "why was", "why is that", "what was wrong", "mistake", "blunder",
    "ultima jugada", "mi jugada", "esa jugada", "jugada anterior", "por que fue", "por que es malo", "por que es mala",
    "que estuvo mal", "que hice mal", "error", "fallo", "me equivoque",
];
const BEST_MOVE: &[&str] = &[
    "best move", "what should", "what do i", "what to play", "next move", "suggest", "hint", "which move", "what move", "help me",
    "mejor jugada", "mejor movimiento", "que debo hacer", "que deberia", "que hago", "que juego", "que jugar", "que muevo",
    "siguiente jugada", "sugier", "sugerencia", "pista", "ayuda", "que jugada", "cual jugada", "que movimiento",
];
const EVAL: &[&str] = &[
    "winning", "who is better", "who's better", "evaluation", "eval", "advantage", "score", "am i",
    "ganando", "quien va mejor", "quien esta mejor", "quien gana", "evaluacion", "ventaja", "como voy", "voy bien", "voy mal",
];
const THREATS: &[&str] = &["threat", "danger", "attack", "hanging", "safe", "amenaza", "peligro", "ataque", "atacad", "colgad", "a salvo", "segur"];
const OPENING: &[&str] = &["opening", "apertura"];
const ENDGAME: &[&str] = &["endgame", "final"];
const PLAN: &[&str] = &[
    "plan", "idea", "strategy", "what now", "explain", "position", "what's going on", "what is going on", "understand",
    "estrategia", "y ahora", "ahora que", "explica", "posicion", "que pasa", "que esta pasando", "entender", "entiendo",
];
const GREETING: &[&str] = &["hello", "hi ", "hey", "thanks", "thank you", "hola", "gracias", "buenas", "buenos dias"];

fn bullets(items: &[String]) -> String {
    items.iter().map(|i| format!("• {i}")).collect::<Vec<_>>().join("\n")
}

fn best_move_text(pos: &Chess, lines: &[EngineLine], p: &Picker, lang: Lang) -> Option<String> {
    let line = lines.first()?;
    let first = line.moves.first()?;
    let m = tactics::parse_any_move(pos, first)?;
    let san = tactics::san(pos, &m);
    let comment = plain_move_comment(pos, &m, lang);
    let cont: Vec<&str> = line.moves.iter().skip(1).take(4).map(String::as_str).collect();
    let lead = p.pick(
        71,
        tl(lang, &["The engine's top choice is", "I'd play", "The strongest move here is"], &["La primera opción del motor es", "Yo jugaría", "La jugada más fuerte aquí es"]),
    );
    let mut s = format!("{lead} **{san}**");
    if !line.score_text.is_empty() {
        s.push_str(&format!(" ({})", line.score_text));
    }
    s.push('.');
    if !comment.quiet {
        s.push(' ');
        s.push_str(&comment.text);
    }
    if !cont.is_empty() {
        s.push_str(&fill(t(lang, " A likely continuation: {c}.", " Una continuación probable: {c}."), &kv(&[("c", &cont.join(" "))])));
    }
    if let Some(second) = lines.get(1).and_then(|l| l.moves.first()) {
        if let Some(m2) = tactics::parse_any_move(pos, second) {
            s.push_str(&fill(
                t(lang, " **{m}** is a decent alternative.", " **{m}** es una alternativa decente."),
                &kv(&[("m", &tactics::san(pos, &m2))]),
            ));
        }
    }
    Some(s)
}

fn hint_without_engine(pos: &Chess, lang: Lang) -> String {
    if let Some(m) = tactics::mate_in_one(pos) {
        return fill(t(lang, "There's a checkmate in one: **{m}**!", "¡Hay un mate en una: **{m}**!"), &kv(&[("m", &tactics::san(pos, &m))]));
    }
    let opp = tactics::hanging_pieces(pos.board(), !pos.turn());
    if let Some(h) = opp.first() {
        if let Some(c) = tactics::best_capture_on(pos, h.square) {
            let mut v = PieceRef::on(h.role, h.square).vars("p", lang);
            v.extend(kv(&[("c", &tactics::san(pos, &c))]));
            return fill(t(lang, "Look at {p} — **{c}** wins material.", "Fíjate en {p}: **{c}** gana material."), &v);
        }
    }
    let own = tactics::hanging_pieces(pos.board(), pos.turn());
    if let Some(h) = own.first() {
        let mut v = PieceRef::new(h.role).vars("p", lang);
        v.extend(kv(&[("sq", &h.square.to_string())]));
        return fill(
            t(lang, "First, take care of your {p_n} on {sq} — it's under attack.", "Primero, ocúpate de tu {p_n} en {sq}: está atacad{p_o}."),
            &v,
        );
    }
    t(
        lang,
        "Start with a quick safety check: any checks, captures or threats? If not, improve your worst-placed piece.",
        "Empieza con un repaso de seguridad: ¿hay jaques, capturas o amenazas? Si no, mejora tu pieza peor colocada.",
    )
    .to_string()
}

/// Build a full rule-based answer in `lang`.
pub(crate) fn answer(req: &ChatRequest, lang: Lang) -> String {
    let q = normalize(&req.question);
    let pos = match gm_engine::parse_fen(&req.fen) {
        Ok(p) => p,
        Err(_) => {
            return t(
                lang,
                "I couldn't read the current position, so let me give general advice: before each move, look for checks, captures and threats, then develop your pieces, control the center and keep your king safe.",
                "No he podido leer la posición actual, así que te doy un consejo general: antes de cada jugada, busca jaques, capturas y amenazas; después desarrolla tus piezas, controla el centro y mantén tu rey a salvo.",
            )
            .into();
        }
    };
    let p = Picker::new(&[&req.fen, &req.question]);
    let lines: Vec<EngineLine> = req.engine_lines.iter().take(5).filter_map(|l| parse_engine_line(l)).collect();
    let ideas = describe::describe(&pos, lang);
    let texts: Vec<String> = ideas.iter().map(|i| i.text.clone()).collect();

    if pos.is_game_over() {
        return bullets(&texts);
    }

    // A specific move named in the question.
    let wants_last = has_any(&q, LAST_MOVE);
    if let Some(m) = find_move_in_question(&pos, &req.question, lang) {
        let comment = plain_move_comment(&pos, &m, lang);
        let san = tactics::san(&pos, &m);
        let mut out = comment.text;
        if let Some(top) = lines.first().and_then(|l| l.moves.first()).and_then(|t| tactics::parse_any_move(&pos, t)) {
            let tail = if top == m {
                fill(t(lang, " Good news: **{m}** is the engine's top choice!", " ¡Buenas noticias: **{m}** es la primera opción del motor!"), &kv(&[("m", &san)]))
            } else {
                let top_san = tactics::san(&pos, &top);
                let tpl = if comment.bad {
                    t(lang, " Instead, consider **{m}**.", " En su lugar, considera **{m}**.")
                } else {
                    t(lang, " The engine slightly prefers **{m}**.", " El motor prefiere ligeramente **{m}**.")
                };
                fill(tpl, &kv(&[("m", &top_san)]))
            };
            out.push_str(&tail);
        }
        return out;
    }
    if wants_last {
        if let Some((prev, m)) = previous_position(req, &pos) {
            let comment = plain_move_comment(&prev, &m, lang);
            let mut out = comment.text;
            if !comment.bad {
                out.push_str(t(
                    lang,
                    " I don't see an immediate tactical problem with it — if the engine disagrees, it's probably a deeper positional issue. Use the Best button to compare.",
                    " No le veo ningún problema táctico inmediato: si el motor no está de acuerdo, seguramente sea algo posicional más profundo. Usa el botón de mejor jugada para comparar.",
                ));
            }
            if let Some(b) = best_move_text(&pos, &lines, &p, lang) {
                out.push_str(t(lang, "\n\nNow: ", "\n\nAhora: "));
                out.push_str(&b);
            }
            return out;
        }
    }

    if has_any(&q, BEST_MOVE) {
        return match best_move_text(&pos, &lines, &p, lang) {
            Some(b) => b,
            None => hint_without_engine(&pos, lang),
        };
    }

    if has_any(&q, EVAL) {
        let mut out = lines.first().and_then(|l| eval_words(l, lang)).unwrap_or_else(|| {
            t(lang, "I don't have an engine evaluation right now, but here's what I see:", "Ahora mismo no tengo la evaluación del motor, pero esto es lo que veo:").to_string()
        });
        out.push('\n');
        out.push_str(&bullets(&texts.iter().take(3).cloned().collect::<Vec<_>>()));
        return out;
    }

    if has_any(&q, THREATS) {
        let threats: Vec<String> = ideas.iter().filter(|i| i.threat).map(|i| i.text.clone()).collect();
        if threats.is_empty() {
            let tpl = t(
                lang,
                "No immediate threats that I can see — {S} is free to improve the position. {hint}",
                "No veo amenazas inmediatas: {s} pueden mejorar su posición con calma. {hint}",
            );
            return fill(
                tpl,
                &kv(&[
                    ("S", &capitalize(side_word(pos.turn(), lang))),
                    ("s", side_word(pos.turn(), lang)),
                    ("hint", &hint_without_engine(&pos, lang)),
                ]),
            );
        }
        return bullets(&threats);
    }

    let ph = describe::phase(&pos);
    if has_any(&q, OPENING) && ph == Phase::Opening {
        let items: Vec<String> = tl(
            lang,
            &[
                "Control the center with pawns and pieces (e4, d4, Nf3, Nc3...).",
                "Develop knights and bishops before moving the queen out.",
                "Castle early to keep the king safe and connect the rooks.",
            ],
            &[
                "Controla el centro con peones y piezas (e4, d4, Nf3, Nc3...).",
                "Desarrolla caballos y alfiles antes de sacar la dama.",
                "Enroca pronto para poner el rey a salvo y conectar las torres.",
            ],
        )
        .iter()
        .map(|s| s.to_string())
        .collect();
        return format!("{}\n{}", t(lang, "In the opening, three things matter most:", "En la apertura, lo que más importa son tres cosas:"), bullets(&items));
    }
    if has_any(&q, ENDGAME) && ph == Phase::Endgame {
        let items: Vec<String> = tl(
            lang,
            &[
                "Activate the king — in the endgame it's a fighting piece.",
                "Create a passed pawn and push it with support.",
                "Rooks belong behind passed pawns (yours or the opponent's).",
            ],
            &[
                "Activa el rey: en el final es una pieza de combate.",
                "Crea un peón pasado y avánzalo con apoyo.",
                "Las torres van detrás de los peones pasados (tuyos o del rival).",
            ],
        )
        .iter()
        .map(|s| s.to_string())
        .collect();
        return bullets(&items);
    }

    if has_any(&q, PLAN) {
        let mut out = String::new();
        if let Some(e) = lines.first().and_then(|l| eval_words(l, lang)) {
            out.push_str(&e);
            out.push('\n');
        }
        out.push_str(&bullets(&texts));
        if let Some(b) = best_move_text(&pos, &lines, &p, lang) {
            out.push_str("\n\n");
            out.push_str(&b);
        }
        return out;
    }

    if has_any(&q, GREETING) || q.trim() == "hi" {
        return p
            .pick(
                72,
                tl(
                    lang,
                    &[
                        "Hi! I'm Mentor Mira. Ask me about the best move, the plan, threats, or why a move was good or bad.",
                        "Happy to help! Try asking \"What's the plan?\" or \"Why is Nf3 good here?\"",
                    ],
                    &[
                        "¡Hola! Soy Mentor Mira. Pregúntame por la mejor jugada, el plan, las amenazas o por qué una jugada fue buena o mala.",
                        "¡Encantada de ayudarte! Prueba a preguntar «¿Cuál es el plan?» o «¿Por qué es buena Nf3 aquí?».",
                    ],
                ),
            )
            .to_string();
    }

    // Default overview.
    let mut out = String::new();
    if let Some(e) = lines.first().and_then(|l| eval_words(l, lang)) {
        out.push_str(&e);
        out.push(' ');
    }
    out.push_str(&fill(
        t(lang, "It's {S}'s move. Here's what stands out:\n", "Juegan {s}. Esto es lo que destaca:\n"),
        &kv(&[("S", side_word(pos.turn(), Lang::En)), ("s", side_word(pos.turn(), lang))]),
    ));
    out.push_str(&bullets(&texts.iter().take(4).cloned().collect::<Vec<_>>()));
    if let Some(b) = best_move_text(&pos, &lines, &p, lang) {
        out.push_str("\n\n");
        out.push_str(&b);
    }
    out
}

/// Side-to-move helper used by the LLM prompt (always English: the prompt is English).
pub(crate) fn side_name(c: Color) -> &'static str {
    tactics::color_name(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_engine_lines() {
        let l = parse_engine_line("+0.45: Nf3 Nc6 Bb5").expect("line");
        assert_eq!(l.pawns, Some(0.45));
        assert_eq!(l.moves, vec!["Nf3", "Nc6", "Bb5"]);
        let l = parse_engine_line("-M3: Qh4+ g3 Qxg3#").expect("line");
        assert_eq!(l.mate, Some(-3));
        let l = parse_engine_line("#2: 1. Qh5 Nf6 2. Qxf7#").expect("line");
        assert_eq!(l.mate, Some(2));
        assert_eq!(l.moves, vec!["Qh5", "Nf6", "Qxf7#"]);
    }
}
