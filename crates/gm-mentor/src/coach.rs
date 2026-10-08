//! Rule-based chat answers (used when no LLM key is configured or the LLM call fails).

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

/// Plain-language verdict for a White-POV engine line.
pub(crate) fn eval_words(line: &EngineLine) -> Option<String> {
    if let Some(m) = line.mate {
        let side = if m > 0 { "White" } else { "Black" };
        return Some(format!("{side} has a forced checkmate in {}.", m.abs()));
    }
    let v = line.pawns?;
    let (side, a) = if v >= 0.0 { ("White", v) } else { ("Black", -v) };
    Some(if a < 0.3 {
        format!("The position is roughly equal ({}).", line.score_text)
    } else if a < 1.0 {
        format!("{side} is slightly better ({}).", line.score_text)
    } else if a < 2.5 {
        format!("{side} is clearly better ({}).", line.score_text)
    } else {
        format!("{side} is winning ({}).", line.score_text)
    })
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

fn find_move_in_question(pos: &Chess, q: &str) -> Option<shakmaty::Move> {
    for raw in q.split(|c: char| c.is_whitespace() || matches!(c, ',' | '?' | '"' | '\'' | '(' | ')' | ';')) {
        let tok = raw.trim_matches(|c: char| c == '.' || c == '!' || c == ':');
        let tok = tok.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        if tok.len() < 2 || tok.len() > 8 {
            continue;
        }
        // Avoid treating plain words as moves: SAN starts with a piece letter, a file, or O.
        let first = tok.chars().next().unwrap_or(' ');
        if !(matches!(first, 'K' | 'Q' | 'R' | 'B' | 'N' | 'O' | '0') || ('a'..='h').contains(&first)) {
            continue;
        }
        if tok.chars().all(|c| c.is_ascii_alphabetic()) && !tok.starts_with('O') && tok.len() > 3 {
            continue; // an English word like "bishop"
        }
        if let Some(m) = tactics::parse_any_move(pos, tok) {
            return Some(m);
        }
    }
    None
}

fn has_any(q: &str, words: &[&str]) -> bool {
    words.iter().any(|w| q.contains(w))
}

fn bullets(items: &[String]) -> String {
    items.iter().map(|i| format!("• {i}")).collect::<Vec<_>>().join("\n")
}

fn best_move_text(pos: &Chess, lines: &[EngineLine], p: &Picker) -> Option<String> {
    let line = lines.first()?;
    let first = line.moves.first()?;
    let m = tactics::parse_any_move(pos, first)?;
    let san = tactics::san(pos, &m);
    let (_, comment) = plain_move_comment(pos, &m);
    let cont: Vec<&str> = line.moves.iter().skip(1).take(4).map(String::as_str).collect();
    let lead = p.pick(71, &["The engine's top choice is", "I'd play", "The strongest move here is"]);
    let mut s = format!("{lead} **{san}**");
    if !line.score_text.is_empty() {
        s.push_str(&format!(" ({})", line.score_text));
    }
    s.push('.');
    if !comment.contains("quiet move") {
        s.push(' ');
        s.push_str(&comment);
    }
    if !cont.is_empty() {
        s.push_str(&format!(" A likely continuation: {}.", cont.join(" ")));
    }
    if let Some(second) = lines.get(1).and_then(|l| l.moves.first()) {
        if let Some(m2) = tactics::parse_any_move(pos, second) {
            s.push_str(&format!(" **{}** is a decent alternative.", tactics::san(pos, &m2)));
        }
    }
    Some(s)
}

fn hint_without_engine(pos: &Chess) -> String {
    if let Some(m) = tactics::mate_in_one(pos) {
        return format!("There's a checkmate in one: **{}**!", tactics::san(pos, &m));
    }
    let opp = tactics::hanging_pieces(pos.board(), !pos.turn());
    if let Some(h) = opp.first() {
        if let Some(c) = tactics::best_capture_on(pos, h.square) {
            return format!("Look at the {} on {} — **{}** wins material.", tactics::role_name(h.role), h.square, tactics::san(pos, &c));
        }
    }
    let own = tactics::hanging_pieces(pos.board(), pos.turn());
    if let Some(h) = own.first() {
        return format!("First, take care of your {} on {} — it's under attack.", tactics::role_name(h.role), h.square);
    }
    "Start with a quick safety check: any checks, captures or threats? If not, improve your worst-placed piece.".to_string()
}

/// Build a full rule-based answer.
pub(crate) fn answer(req: &ChatRequest) -> String {
    let q = req.question.to_lowercase();
    let pos = match gm_engine::parse_fen(&req.fen) {
        Ok(p) => p,
        Err(_) => {
            return "I couldn't read the current position, so let me give general advice: before each move, look for checks, captures and threats, then develop your pieces, control the center and keep your king safe.".into();
        }
    };
    let p = Picker::new(&[&req.fen, &req.question]);
    let lines: Vec<EngineLine> = req.engine_lines.iter().take(5).filter_map(|l| parse_engine_line(l)).collect();
    let ideas = describe::describe(&pos);
    let stm = tactics::color_name(pos.turn());

    if pos.is_game_over() {
        return bullets(&ideas);
    }

    // A specific move named in the question.
    let wants_last = has_any(&q, &["last move", "my move", "that move", "previous move", "why was", "why is that", "what was wrong", "mistake", "blunder"]);
    let named = find_move_in_question(&pos, &req.question);
    if let Some(m) = named {
        let (bad, comment) = plain_move_comment(&pos, &m);
        let san = tactics::san(&pos, &m);
        let mut out = comment;
        if let Some(top) = lines.first().and_then(|l| l.moves.first()).and_then(|t| tactics::parse_any_move(&pos, t)) {
            if top == m {
                out.push_str(&format!(" Good news: **{san}** is the engine's top choice!"));
            } else {
                let top_san = tactics::san(&pos, &top);
                if bad {
                    out.push_str(&format!(" Instead, consider **{top_san}**."));
                } else {
                    out.push_str(&format!(" The engine slightly prefers **{top_san}**."));
                }
            }
        }
        return out;
    }
    if wants_last {
        if let Some((prev, m)) = previous_position(req, &pos) {
            let (bad, comment) = plain_move_comment(&prev, &m);
            let mut out = comment;
            if !bad {
                out.push_str(" I don't see an immediate tactical problem with it — if the engine disagrees, it's probably a deeper positional issue. Use the Best button to compare.");
            }
            if let Some(b) = best_move_text(&pos, &lines, &p) {
                out.push_str("\n\nNow: ");
                out.push_str(&b);
            }
            return out;
        }
    }

    if has_any(&q, &["best move", "what should", "what do i", "what to play", "next move", "suggest", "hint", "which move", "what move", "help me"]) {
        return match best_move_text(&pos, &lines, &p) {
            Some(b) => b,
            None => hint_without_engine(&pos),
        };
    }

    if has_any(&q, &["winning", "who is better", "who's better", "evaluation", "eval", "advantage", "score", "am i"]) {
        let mut out = lines
            .first()
            .and_then(eval_words)
            .unwrap_or_else(|| "I don't have an engine evaluation right now, but here's what I see:".to_string());
        out.push('\n');
        out.push_str(&bullets(&ideas.iter().take(3).cloned().collect::<Vec<_>>()));
        return out;
    }

    if has_any(&q, &["threat", "danger", "attack", "hanging", "safe"]) {
        let threats: Vec<String> = ideas
            .iter()
            .filter(|i| i.contains("threat") || i.contains("attack") || i.contains("undefended") || i.contains("under-protected") || i.contains("check") || i.contains("exposed"))
            .cloned()
            .collect();
        if threats.is_empty() {
            return format!("No immediate threats that I can see — {stm} is free to improve the position. {}", hint_without_engine(&pos));
        }
        return bullets(&threats);
    }

    let ph = describe::phase(&pos);
    if has_any(&q, &["opening"]) && ph == Phase::Opening {
        return format!(
            "{}\n{}",
            "In the opening, three things matter most:",
            bullets(&[
                "Control the center with pawns and pieces (e4, d4, Nf3, Nc3...).".into(),
                "Develop knights and bishops before moving the queen out.".into(),
                "Castle early to keep the king safe and connect the rooks.".into(),
            ])
        );
    }
    if has_any(&q, &["endgame"]) && ph == Phase::Endgame {
        return bullets(&[
            "Activate the king — in the endgame it's a fighting piece.".into(),
            "Create a passed pawn and push it with support.".into(),
            "Rooks belong behind passed pawns (yours or the opponent's).".into(),
        ]);
    }

    if has_any(&q, &["plan", "idea", "strategy", "what now", "explain", "position", "what's going on", "what is going on", "understand"]) {
        let mut out = String::new();
        if let Some(e) = lines.first().and_then(eval_words) {
            out.push_str(&e);
            out.push('\n');
        }
        out.push_str(&bullets(&ideas));
        if let Some(b) = best_move_text(&pos, &lines, &p) {
            out.push_str("\n\n");
            out.push_str(&b);
        }
        return out;
    }

    if has_any(&q, &["hello", "hi ", "hey", "thanks", "thank you"]) || q.trim() == "hi" {
        return p
            .pick(72, &[
                "Hi! I'm Mentor Mira. Ask me about the best move, the plan, threats, or why a move was good or bad.",
                "Happy to help! Try asking \"What's the plan?\" or \"Why is Nf3 good here?\"",
            ])
            .to_string();
    }

    // Default overview.
    let mut out = String::new();
    if let Some(e) = lines.first().and_then(eval_words) {
        out.push_str(&e);
        out.push(' ');
    }
    out.push_str(&format!("It's {stm}'s move. Here's what stands out:\n"));
    out.push_str(&bullets(&ideas.iter().take(4).cloned().collect::<Vec<_>>()));
    if let Some(b) = best_move_text(&pos, &lines, &p) {
        out.push_str("\n\n");
        out.push_str(&b);
    }
    out
}

/// Side-to-move helper used by the LLM prompt.
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
