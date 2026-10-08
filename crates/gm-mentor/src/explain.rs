//! Rule-based move explanations in the style of a friendly human coach.

use gm_engine::Score;
use shakmaty::{Chess, Color, Position, Role};

use crate::phrase::{article, capitalize, fill, Picker};
use crate::tactics::{self, value, LineKind, MoveFacts};
use crate::MoveContext;

/// Move quality bucket derived from the classification string (or from evals when missing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cat {
    Brilliant,
    Great,
    Best,
    Good,
    Book,
    Forced,
    Inaccuracy,
    Mistake,
    Miss,
    Blunder,
}

impl Cat {
    fn is_bad(self) -> bool {
        matches!(self, Cat::Inaccuracy | Cat::Mistake | Cat::Miss | Cat::Blunder)
    }
}

/// Score in centipawns from `side`'s point of view; mates are mapped to +-100000 (nearer = bigger).
pub(crate) fn pov_cp(score: Score, side: Color) -> i32 {
    let white = match score {
        Score::Cp(c) => c.clamp(-50_000, 50_000),
        Score::Mate(m) if m > 0 => 100_000 - m.min(1000),
        Score::Mate(m) if m < 0 => -100_000 - m.max(-1000),
        Score::Mate(_) => 0,
    };
    match side {
        Color::White => white,
        Color::Black => -white,
    }
}

/// Mate distance for `side` (positive = `side` mates).
pub(crate) fn pov_mate(score: Score, side: Color) -> Option<i32> {
    match score {
        Score::Mate(m) if m != 0 => Some(if side == Color::White { m } else { -m }),
        _ => None,
    }
}

fn category(cls: &str, drop: i32) -> Cat {
    match cls.trim().to_ascii_lowercase().as_str() {
        "brilliant" => Cat::Brilliant,
        "great" => Cat::Great,
        "best" => Cat::Best,
        "excellent" | "good" => Cat::Good,
        "book" => Cat::Book,
        "forced" => Cat::Forced,
        "inaccuracy" => Cat::Inaccuracy,
        "mistake" => Cat::Mistake,
        "miss" => Cat::Miss,
        "blunder" => Cat::Blunder,
        _ => match drop {
            d if d <= 10 => Cat::Best,
            d if d <= 50 => Cat::Good,
            d if d <= 100 => Cat::Inaccuracy,
            d if d <= 250 => Cat::Mistake,
            _ => Cat::Blunder,
        },
    }
}

fn piece_on(role: shakmaty::Role, sq: shakmaty::Square) -> String {
    format!("the {} on {}", tactics::role_name(role), sq)
}

/// Positive verb phrases ("develops the knight ...") describing what a move achieves, best first.
pub(crate) fn positive_reasons(f: &MoveFacts, p: &Picker) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if f.is_mate {
        out.push("delivers checkmate".into());
        return out;
    }
    if f.fork.len() >= 2 {
        let a = f.fork[0];
        let b = f.fork[1];
        let name = |(sq, r): (shakmaty::Square, Role)| {
            if r == Role::King {
                "the king".to_string()
            } else {
                piece_on(r, sq)
            }
        };
        let t = p.pick(11, &[
            "forks {a} and {b} — they can't both be saved",
            "attacks {a} and {b} at the same time, a classic fork",
            "creates a fork, hitting {a} and {b} at once",
        ]);
        out.push(fill(t, &[("a", &name(a)), ("b", &name(b))]));
    }
    if let Some(m) = &f.stops_mate {
        let t = p.pick(25, &["stops the threat of {m}", "defends against {m}, which was threatening mate", "shuts down the mating threat {m}"]);
        out.push(fill(t, &[("m", m)]));
    }
    if let Some(cap) = f.captured {
        if f.wins_material() {
            let n = tactics::role_name(cap);
            if f.material_lead < 0 {
                let t = p.pick(12, &["wins back the {n}", "recaptures the {n} and restores the balance", "takes back the {n}"]);
                out.push(fill(t, &[("n", n)]));
            } else if !f.recapturable && f.capture_net >= value(cap) - 10 {
                let t = p.pick(12, &["wins {art} {n} for free", "picks up a free {n}", "grabs the undefended {n}"]);
                out.push(fill(t, &[("art", article(n)), ("n", n)]));
            } else if f.capture_net >= value(cap) - 30 {
                out.push(format!("wins {} {n}", article(n)));
            } else {
                out.push("comes out ahead in the exchange".to_string());
            }
        }
    }
    if let Some(l) = f.lines.first() {
        let front = if l.front.1 == Role::King { "the king".to_string() } else { piece_on(l.front.1, l.front.0) };
        let behind = if l.behind.1 == Role::King { "the king".to_string() } else { piece_on(l.behind.1, l.behind.0) };
        match l.kind {
            LineKind::Pin => {
                let t = p.pick(13, &["pins {f} to {b}", "sets up a pin: {f} can't move without exposing {b}"]);
                out.push(fill(t, &[("f", &front), ("b", &behind)]));
            }
            LineKind::Skewer => {
                let t = p.pick(14, &["skewers {f} and {b} behind it", "lines up a skewer through {f} to {b}"]);
                out.push(fill(t, &[("f", &front), ("b", &behind)]));
            }
        }
    }
    if let Some(r) = f.promotion {
        out.push(format!("promotes the pawn to {} {}", article(tactics::role_name(r)), tactics::role_name(r)));
    }
    if let Some(m) = &f.threatens_mate {
        let t = p.pick(15, &["threatens {m} with checkmate", "sets up a mating threat ({m})", "creates the threat of {m}, checkmate"]);
        out.push(fill(t, &[("m", m)]));
    }
    if f.is_trade() {
        if f.material_lead >= 2 {
            let t = p.pick(17, &[
                "trades pieces — a great idea when you're ahead in material",
                "simplifies the position, which helps the side that's ahead",
            ]);
            out.push(t.to_string());
        } else {
            let n = f.captured.map(tactics::role_name).unwrap_or("piece");
            let t = p.pick(18, &["keeps the material balanced by trading {n}s", "recaptures and keeps material level", "completes a fair trade of {n}s"]);
            out.push(fill(t, &[("n", n)]));
        }
    }
    if let Some((sq, r)) = f.new_threat {
        if r == Role::Queen {
            out.push(format!("attacks the queen on {sq}"));
        } else {
            let t = p.pick(16, &["attacks {p}, which is now in trouble", "goes after {p}", "puts pressure on {p}"]);
            out.push(fill(t, &[("p", &piece_on(r, sq))]));
        }
    }
    if let Some((sq, r)) = f.defends.filter(|_| f.captured.is_none()) {
        let t = p.pick(26, &["protects {p}", "takes care of {p}, which was under attack"]);
        out.push(fill(t, &[("p", &piece_on(r, sq))]));
    }
    if f.saves_piece {
        let n = tactics::role_name(f.role);
        let t = p.pick(19, &["moves the {n} out of danger", "rescues the attacked {n}", "gets the {n} to safety"]);
        out.push(fill(t, &[("n", n)]));
    }
    if let Some(_side) = f.castle {
        let t = p.pick(20, &[
            "castles, tucking the king into safety and connecting the rooks",
            "gets the king safe and brings a rook toward the center",
            "castles — king safety first, and the rooks can now work together",
        ]);
        out.push(t.to_string());
    }
    if f.develops {
        let n = tactics::role_name(f.role);
        let t = p.pick(21, &[
            "develops the {n} toward the center",
            "brings the {n} into the game",
            "gets another piece into play",
        ]);
        out.push(fill(t, &[("n", n)]));
    }
    if f.center {
        let t = if f.role == Role::Pawn {
            p.pick(22, &["claims space in the center", "stakes a claim in the center", "grabs a big share of the center"])
        } else {
            p.pick(23, &["controls important central squares", "eyes the center"])
        };
        out.push(t.to_string());
    }
    if f.gives_check && out.is_empty() {
        out.push(p.pick(24, &["gives check and keeps the opponent busy", "checks the king and keeps the initiative"]).to_string());
    }
    out
}

/// Negative full sentences describing what's wrong with the played move.
fn problems(ctx: &MoveContext, f: &MoveFacts, best: Option<&MoveFacts>, cat: Cat, p: &Picker) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mover = f.mover;
    if let Some((m, back_rank)) = &f.allows_mate {
        if *back_rank {
            out.push(fill(
                p.pick(31, &[
                    "This allows a back-rank mate with {m} — the king has no escape squares.",
                    "Careful: {m} is now a back-rank checkmate, since the king is boxed in by its own pawns.",
                ]),
                &[("m", m)],
            ));
        } else {
            out.push(fill(p.pick(32, &["This allows {m}, checkmate!", "Ouch — this lets {m} deliver checkmate."]), &[("m", m)]));
        }
        return out;
    }
    if let Some(m) = &f.missed_mate {
        out.push(fill(p.pick(33, &["You had checkmate in one with {m}!", "There was a mate in one: {m}!"]), &[("m", m)]));
        return out;
    }
    if let (Some(before), after) = (pov_mate(ctx.eval_before, mover), pov_mate(ctx.eval_after, mover)) {
        if before > 0 && after.map(|a| a <= 0).unwrap_or(true) {
            out.push(p.pick(34, &["This lets a forced checkmate slip away.", "There was a forced mate here, and this move misses it."]).to_string());
        }
    }
    if f.is_stalemate && f.material_lead > 0 {
        out.push("This is stalemate — a winning position thrown away as a draw!".into());
        return out;
    }
    if let Some((h, cap, was)) = f.hangs.first() {
        if h.gain >= 90 {
            let piece = piece_on(h.role, h.square);
            let s = if *was {
                p.pick(35, &["This leaves {p} hanging", "This doesn't deal with the threat to {p}", "{P} is still en prise after this"])
            } else if h.undefended {
                p.pick(36, &["This hangs {p}", "This leaves {p} undefended", "This drops {p}"])
            } else {
                p.pick(37, &["This puts {p} where it can be won", "This leaves {p} under-protected"])
            };
            let mut sent = fill(s, &[("p", &piece), ("P", &capitalize(&piece))]);
            if let Some(c) = cap {
                sent.push_str(&format!(" — {c} wins it."));
            } else {
                sent.push('.');
            }
            out.push(sent);
        }
    }
    if out.is_empty() && f.captured.is_some() && f.capture_net <= -90 {
        out.push(
            p.pick(38, &[
                "This capture doesn't add up — after the recapture you lose material.",
                "The trade here costs material once the opponent takes back.",
            ])
            .to_string(),
        );
    }
    if out.is_empty() && cat == Cat::Miss {
        if let Some(b) = best {
            if let Some(cap) = b.captured.filter(|_| b.wins_material()) {
                out.push(format!(
                    "This misses a chance: {} would have won {} {}.",
                    b.san,
                    article(tactics::role_name(cap)),
                    tactics::role_name(cap)
                ));
            } else if b.fork.len() >= 2 {
                out.push(format!("This misses a fork with {}.", b.san));
            } else {
                out.push(p.pick(39, &["This misses a chance to punish the opponent's last move.", "The opponent slipped up, but this doesn't take advantage."]).to_string());
            }
        }
    }
    if out.is_empty() && f.king_walk {
        out.push("Moving the king this early gives up castling and leaves it exposed.".into());
    }
    if out.is_empty() && f.weakens_king {
        out.push("Pushing a pawn in front of the castled king weakens its shelter.".into());
    }
    out
}

fn eval_drop(ctx: &MoveContext, mover: Color) -> i32 {
    let before = pov_cp(ctx.eval_before, mover).clamp(-2000, 2000);
    let after = pov_cp(ctx.eval_after, mover).clamp(-2000, 2000);
    (before - after).max(0)
}

fn generic_bad(cat: Cat, ctx: &MoveContext, mover: Color, p: &Picker) -> String {
    let after = pov_cp(ctx.eval_after, mover);
    match cat {
        Cat::Blunder if after < -150 => p.pick(41, &["This turns the game around for the opponent.", "This seriously damages the position."]),
        Cat::Blunder | Cat::Mistake => p.pick(42, &["This gives away a big part of the advantage.", "This lets the opponent take over.", "This hands the opponent a strong game."]),
        _ => p.pick(43, &["This is a bit slow.", "This is playable, but there was something more active.", "A small slip — the position gets a little harder."]),
    }
    .to_string()
}

fn fallback_text(ctx: &MoveContext, cat: Cat) -> String {
    let played = if ctx.played_san.is_empty() { ctx.played_uci.as_str() } else { ctx.played_san.as_str() };
    let best = if ctx.best_san.is_empty() { ctx.best_uci.as_str() } else { ctx.best_san.as_str() };
    let better = if !best.is_empty() && best != played { format!(" {best} was better.") } else { String::new() };
    match cat {
        Cat::Brilliant => "A brilliant, hard-to-find move!".into(),
        Cat::Great => "A great move — the key idea in this position.".into(),
        Cat::Best => "That's the best move here.".into(),
        Cat::Good => format!("A solid move.{better}"),
        Cat::Book => "A well-known opening move.".into(),
        Cat::Forced => "This was the only reasonable move.".into(),
        Cat::Inaccuracy => format!("A little imprecise.{better}"),
        Cat::Mistake => format!("This is a mistake.{better}"),
        Cat::Miss => format!("This misses a chance.{better}"),
        Cat::Blunder => format!("This is a serious error.{better}"),
    }
}

fn sentence(clause: &str) -> String {
    let c = clause.trim_end_matches('.');
    format!("{}.", c)
}

pub fn explain_move(ctx: &MoveContext) -> String {
    let Ok(pos) = gm_engine::parse_fen(&ctx.fen_before) else {
        return fallback_text(ctx, category(&ctx.classification, 0));
    };
    let played = tactics::parse_any_move(&pos, &ctx.played_uci).or_else(|| tactics::parse_any_move(&pos, &ctx.played_san));
    let Some(played) = played else {
        return fallback_text(ctx, category(&ctx.classification, 0));
    };
    let mover = pos.turn();
    let cat = category(&ctx.classification, eval_drop(ctx, mover));
    let p = Picker::new(&[&ctx.fen_before, &ctx.played_uci, &ctx.played_san]);
    let f = tactics::move_facts(&pos, &played);

    let best_move = tactics::parse_any_move(&pos, &ctx.best_uci).or_else(|| tactics::parse_any_move(&pos, &ctx.best_san));
    let best_is_played = best_move.as_ref().map(|b| *b == played).unwrap_or(true);
    let best_facts = match &best_move {
        Some(b) if !best_is_played => Some(tactics::move_facts(&pos, b)),
        _ => None,
    };

    explain_with(ctx, &pos, cat, &f, best_facts.as_ref(), &p)
}

/// Engine-free verdict on a move: (is_problematic, sentence). Used by the chat fallback.
pub(crate) fn plain_move_comment(pos: &Chess, m: &shakmaty::Move) -> (bool, String) {
    let f = tactics::move_facts(pos, m);
    let fen = gm_engine::to_fen(pos);
    let p = Picker::new(&[&fen, &f.san]);
    if f.is_mate {
        return (false, format!("**{}** is checkmate!", f.san));
    }
    let ctx = MoveContext { fen_before: fen.clone(), ..Default::default() };
    let probs = problems(&ctx, &f, None, Cat::Mistake, &p);
    if let Some(first) = probs.into_iter().next() {
        return (true, first.replacen("This", &format!("**{}**", f.san), 1));
    }
    let reasons = positive_reasons(&f, &p);
    match reasons.first() {
        Some(r) => (false, format!("**{}** {r}.", f.san)),
        None => (false, format!("**{}** is a quiet move — it doesn't create or allow any immediate tactics.", f.san)),
    }
}

fn better_clause(b: &MoveFacts, p: &Picker) -> String {
    let reasons = positive_reasons(b, p);
    let opener = p.pick(51, &["Better was", "Stronger was", "The engine prefers"]);
    if b.is_mate {
        return format!("{opener} {}, which is checkmate!", b.san);
    }
    match reasons.first() {
        Some(r) if r.contains("which") || r.contains(',') => format!("{opener} {} — it {r}.", b.san),
        Some(r) => format!("{opener} {}, which {r}.", b.san),
        None => format!("{opener} {}.", b.san),
    }
}

fn explain_with(ctx: &MoveContext, pos: &Chess, cat: Cat, f: &MoveFacts, best: Option<&MoveFacts>, p: &Picker) -> String {
    let mover = f.mover;
    if f.is_mate {
        return p
            .pick(1, &["Checkmate! Beautifully finished.", "Checkmate — game over. Well played!", "That's checkmate! Nicely done."])
            .to_string();
    }

    if cat.is_bad() {
        let mut parts = problems(ctx, f, best, cat, p);
        if parts.is_empty() {
            parts.push(generic_bad(cat, ctx, mover, p));
        }
        parts.truncate(2);
        if let Some(b) = best.filter(|b| !parts.iter().any(|t| t.contains(&b.san))) {
            parts.push(better_clause(b, p));
        }
        return parts.join(" ");
    }

    let reasons = positive_reasons(f, p);
    let reason_sentence = |lead: &str| -> Option<String> {
        let mut rs = reasons.iter().take(2);
        let first = rs.next()?;
        let mut s = format!("{lead} {first}");
        let simple = |c: &str| !c.contains(',') && !c.contains('—') && !c.contains(':') && !c.contains(" and ");
        if let Some(second) = rs.next().filter(|sec| simple(first) && simple(sec)) {
            s.push_str(" and ");
            s.push_str(second);
        }
        Some(sentence(&s))
    };

    match cat {
        Cat::Forced => {
            if pos.legal_moves().len() == 1 {
                "This was the only legal move.".into()
            } else {
                p.pick(2, &["This was the only move that holds things together.", "Forced — everything else loses."]).to_string()
            }
        }
        Cat::Book => {
            let lead = p.pick(3, &["A well-known opening move.", "Straight out of opening theory.", "A book move — this is how the masters play it."]);
            match reason_sentence("It") {
                Some(r) => format!("{lead} {r}"),
                None => lead.to_string(),
            }
        }
        Cat::Brilliant => {
            let sac = f.hangs.first().map(|(h, _, _)| h.role).or((f.capture_net < 0).then_some(f.role));
            let lead = match sac {
                Some(r) => format!(
                    "{} Giving up the {} is the key idea.",
                    p.pick(4, &["Brilliant!", "Wow — brilliant!", "A brilliant sacrifice!"]),
                    tactics::role_name(r)
                ),
                None => p.pick(5, &["Brilliant!", "A brilliant find!"]).to_string(),
            };
            match reason_sentence("This") {
                Some(r) => format!("{lead} {r}"),
                None => lead,
            }
        }
        Cat::Great => {
            let lead = p.pick(6, &["Great find!", "Great move!", "Excellent spot!"]);
            match reason_sentence("This") {
                Some(r) => format!("{lead} {r}"),
                None => format!("{lead} It's the only move that keeps the advantage."),
            }
        }
        Cat::Best => {
            let lead = p.pick(7, &["Nice!", "Well played!", "Spot on!", "Exactly right!"]);
            match reason_sentence("This") {
                Some(r) => format!("{lead} {r}"),
                None => {
                    let ev = pov_cp(ctx.eval_after, mover);
                    let tail = if ev >= 300 {
                        "This keeps a winning position under control."
                    } else if ev <= -300 {
                        "It's the most stubborn defense available."
                    } else {
                        "It's the engine's top choice here."
                    };
                    format!("{lead} {tail}")
                }
            }
        }
        _ => {
            // Good / excellent.
            let lead = p.pick(8, &["Good move.", "Solid.", "A good, healthy move."]);
            let mut s = match reason_sentence("This") {
                Some(r) => format!("{lead} {r}"),
                None => lead.to_string(),
            };
            if let Some(b) = best {
                s.push(' ');
                s.push_str(&fill(p.pick(9, &["{b} was slightly more precise.", "{b} was a touch stronger.", "The engine slightly prefers {b}."]), &[("b", &b.san)]));
            }
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(fen: &str, played: &str, best: &str, cls: &str, before: Score, after: Score) -> MoveContext {
        MoveContext {
            fen_before: fen.into(),
            played_uci: played.into(),
            best_uci: best.into(),
            eval_before: before,
            eval_after: after,
            classification: cls.into(),
            ..Default::default()
        }
    }

    #[test]
    fn hanging_piece_blunder() {
        // Two Knights: Bc4-a6?? hangs the bishop to bxa6.
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4";
        let text = explain_move(&ctx(fen, "c4a6", "e1g1", "blunder", Score::Cp(30), Score::Cp(-300)));
        assert!(text.contains("bishop on a6"), "{text}");
        assert!(text.contains("bxa6"), "{text}");
        assert!(text.contains("O-O"), "{text}");
    }

    #[test]
    fn fork_praised() {
        // White knight jumps to c7 forking king e8 and rook a8.
        let fen = "r3k3/8/8/1N6/8/8/8/4K3 w - - 0 1";
        let text = explain_move(&ctx(fen, "b5c7", "b5c7", "best", Score::Cp(100), Score::Cp(500)));
        assert!(text.to_lowercase().contains("fork"), "{text}");
        assert!(text.contains("rook on a8"), "{text}");
    }

    #[test]
    fn mate_delivered_and_missed() {
        let fen = "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1";
        let text = explain_move(&ctx(fen, "a1a8", "a1a8", "best", Score::Mate(1), Score::Mate(0)));
        assert!(text.to_lowercase().contains("checkmate"), "{text}");
        let text = explain_move(&ctx(fen, "a1a7", "a1a8", "miss", Score::Mate(1), Score::Cp(500)));
        assert!(text.contains("Ra8#"), "{text}");
    }

    #[test]
    fn allows_back_rank_mate() {
        // Black to move; white threatens Re8# if black ignores it.
        let fen = "6k1/5ppp/8/8/8/8/5PPP/4R1K1 b - - 0 1";
        let text = explain_move(&ctx(fen, "a7a6", "h7h6", "blunder", Score::Cp(0), Score::Mate(1)));
        // a7a6 is illegal here (no pawn) -> fallback still friendly.
        assert!(!text.is_empty());
        let fen = "r5k1/5ppp/8/8/8/8/5PPP/4R1K1 b - - 0 1";
        let text = explain_move(&ctx(fen, "a8a7", "h7h6", "blunder", Score::Cp(0), Score::Mate(1)));
        assert!(text.contains("Re8#"), "{text}");
        assert!(text.contains("back-rank"), "{text}");
    }

    #[test]
    fn development_and_castling() {
        let fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
        let text = explain_move(&ctx(fen, "g1f3", "g1f3", "best", Score::Cp(30), Score::Cp(30)));
        assert!(text.contains("knight"), "{text}");
        let fen = "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4";
        let text = explain_move(&ctx(fen, "e1g1", "e1g1", "book", Score::Cp(30), Score::Cp(30)));
        assert!(text.to_lowercase().contains("king"), "{text}");
    }

    #[test]
    fn garbage_input_does_not_panic() {
        let text = explain_move(&ctx("not a fen", "zz", "", "", Score::Cp(0), Score::Mate(-3)));
        assert!(!text.is_empty());
        let text = explain_move(&MoveContext::default());
        assert!(!text.is_empty());
    }
}
