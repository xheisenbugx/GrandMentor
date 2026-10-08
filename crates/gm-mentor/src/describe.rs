//! Static position description: 3-6 short, beginner-friendly ideas about a position.

use shakmaty::{Bitboard, Board, CastlingSide, Chess, Color, File, Position, Rank, Role, Square};

use crate::phrase::{capitalize, Picker};
use crate::tactics::{self, piece_material, role_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Opening,
    Middlegame,
    Endgame,
}

pub(crate) fn phase(pos: &Chess) -> Phase {
    let b = pos.board();
    let npm = piece_material(b, Color::White) + piece_material(b, Color::Black);
    let minors_home = undeveloped_minors(b, Color::White).len() + undeveloped_minors(b, Color::Black).len();
    if npm <= 2600 || (b.queens().is_empty() && npm <= 3300) {
        Phase::Endgame
    } else if (pos.fullmoves().get() <= 10 && minors_home >= 3) || pos.fullmoves().get() <= 6 {
        Phase::Opening
    } else {
        Phase::Middlegame
    }
}

fn home_minors(c: Color) -> [(Square, Role); 4] {
    match c {
        Color::White => [(Square::B1, Role::Knight), (Square::G1, Role::Knight), (Square::C1, Role::Bishop), (Square::F1, Role::Bishop)],
        Color::Black => [(Square::B8, Role::Knight), (Square::G8, Role::Knight), (Square::C8, Role::Bishop), (Square::F8, Role::Bishop)],
    }
}

pub(crate) fn undeveloped_minors(b: &Board, c: Color) -> Vec<(Square, Role)> {
    home_minors(c)
        .into_iter()
        .filter(|(sq, r)| b.piece_at(*sq).map(|p| p.color == c && p.role == *r).unwrap_or(false))
        .collect()
}

fn file_bb(f: File) -> Bitboard {
    Bitboard::from_file(f)
}

fn adjacent_files(f: File) -> Bitboard {
    let mut bb = Bitboard::EMPTY;
    if let Some(l) = f.offset(-1) {
        bb |= file_bb(l);
    }
    if let Some(r) = f.offset(1) {
        bb |= file_bb(r);
    }
    bb
}

/// Squares strictly in front of `sq` from `c`'s perspective, on files f-1..f+1.
fn front_span(c: Color, sq: Square) -> Bitboard {
    let files = file_bb(sq.file()) | adjacent_files(sq.file());
    let mut ranks = Bitboard::EMPTY;
    let r = sq.rank() as i32;
    for i in 0..8 {
        let ahead = match c {
            Color::White => i > r,
            Color::Black => i < r,
        };
        if ahead {
            ranks |= Bitboard::from_rank(Rank::new(i as u32));
        }
    }
    files & ranks
}

pub(crate) fn passed_pawns(b: &Board, c: Color) -> Vec<Square> {
    let theirs = b.by_color(!c) & b.pawns();
    let mut out = Vec::new();
    for sq in b.by_color(c) & b.pawns() {
        if (front_span(c, sq) & theirs).is_empty() {
            out.push(sq);
        }
    }
    // Most advanced first.
    out.sort_by_key(|s| match c {
        Color::White => -(s.rank() as i32),
        Color::Black => s.rank() as i32,
    });
    out
}

fn isolated_pawns(b: &Board, c: Color) -> Vec<Square> {
    let ours = b.by_color(c) & b.pawns();
    ours.into_iter().filter(|sq| (adjacent_files(sq.file()) & ours).is_empty()).collect()
}

fn doubled_files(b: &Board, c: Color) -> Vec<File> {
    let ours = b.by_color(c) & b.pawns();
    File::ALL.into_iter().filter(|f| (file_bb(*f) & ours).count() >= 2).collect()
}

fn side_label(c: Color, to_move: Color) -> String {
    if c == to_move {
        format!("{} (to move)", tactics::color_name(c))
    } else {
        tactics::color_name(c).to_string()
    }
}

fn material_sentence(b: &Board) -> String {
    let w = tactics::material_points(b, Color::White);
    let bl = tactics::material_points(b, Color::Black);
    let diff = w - bl;
    if diff == 0 {
        // Note piece imbalances even when points are equal.
        let count = |c: Color, r: Role| (b.by_color(c) & b.by_role(r)).count() as i32;
        let wb = count(Color::White, Role::Bishop);
        let bb = count(Color::Black, Role::Bishop);
        if wb == 2 && bb < 2 {
            return "Material is equal, but White has the bishop pair — a long-term plus in open positions.".into();
        }
        if bb == 2 && wb < 2 {
            return "Material is equal, but Black has the bishop pair — a long-term plus in open positions.".into();
        }
        return "Material is level.".into();
    }
    let (leader, n) = if diff > 0 { (Color::White, diff) } else { (Color::Black, -diff) };
    let what = describe_material_edge(b, leader);
    let advice = if n >= 3 {
        "trade pieces (not pawns) and head for a simple endgame"
    } else {
        "keep it safe and look to trade down"
    };
    format!("{} is up {n} point{} of material{what} — the plan is to {advice}.", tactics::color_name(leader), if n == 1 { "" } else { "s" })
}

fn describe_material_edge(b: &Board, leader: Color) -> String {
    let mut extra = Vec::new();
    let mut all_single = true;
    for r in [Role::Queen, Role::Rook, Role::Bishop, Role::Knight, Role::Pawn] {
        let a = (b.by_color(leader) & b.by_role(r)).count() as i32;
        let o = (b.by_color(!leader) & b.by_role(r)).count() as i32;
        if a > o {
            let n = a - o;
            if n == 1 {
                extra.push(role_name(r).to_string());
            } else {
                all_single = false;
                extra.push(format!("{n} {}s", role_name(r)));
            }
        }
    }
    if extra.is_empty() || extra.len() > 3 {
        String::new()
    } else if all_single {
        format!(" (an extra {})", extra.join(" and "))
    } else {
        format!(" (extra: {})", extra.join(", "))
    }
}

fn king_exposed(pos: &Chess, c: Color) -> bool {
    let b = pos.board();
    let Some(k) = b.king_of(c) else { return false };
    let center_files = k.file() >= File::C && k.file() <= File::F;
    let home = match c {
        Color::White => k.rank() <= Rank::Second,
        Color::Black => k.rank() >= Rank::Seventh,
    };
    let can_castle = pos.castles().has(c, CastlingSide::KingSide) || pos.castles().has(c, CastlingSide::QueenSide);
    let enemy_queen = !(b.by_color(!c) & b.queens()).is_empty();
    // Open file in front of the king.
    let own_pawns = b.by_color(c) & b.pawns();
    let open_in_front = (file_bb(k.file()) & own_pawns).is_empty();
    enemy_queen && ((!home) || (center_files && !can_castle && pos.fullmoves().get() >= 10) || (open_in_front && center_files && pos.fullmoves().get() >= 8))
}

fn pawn_shield_weak(b: &Board, c: Color) -> bool {
    let Some(k) = b.king_of(c) else { return false };
    if k.file() > File::C && k.file() < File::G {
        return false;
    }
    let shield_rank = match c {
        Color::White => k.rank().offset(1),
        Color::Black => k.rank().offset(-1),
    };
    let Some(sr) = shield_rank else { return false };
    let files = file_bb(k.file()) | adjacent_files(k.file());
    let near = files & (Bitboard::from_rank(sr) | sr.offset(if c == Color::White { 1 } else { -1 }).map(Bitboard::from_rank).unwrap_or(Bitboard::EMPTY));
    (near & b.by_color(c) & b.pawns()).count() <= 1 && !(b.by_color(!c) & b.queens()).is_empty()
}

/// Rule-based list of ideas (3-6) about a position.
pub fn describe_position(fen: &str) -> Vec<String> {
    let Ok(pos) = gm_engine::parse_fen(fen) else {
        return vec!["That position doesn't look valid — try setting it up again.".to_string()];
    };
    describe(&pos)
}

pub(crate) fn describe(pos: &Chess) -> Vec<String> {
    let b = pos.board();
    let stm = pos.turn();
    let p = Picker::new(&[&gm_engine::to_fen(pos)]);
    let mut ideas: Vec<String> = Vec::new();

    // Game over states.
    if pos.is_checkmate() {
        return vec![
            format!("Checkmate — {} has won the game.", tactics::color_name(!stm)),
            "Step back through the moves to see how the attack came together.".into(),
            "Notice which pieces covered the king's escape squares.".into(),
        ];
    }
    if pos.is_stalemate() {
        return vec![
            "Stalemate — the game is a draw because the side to move has no legal moves but isn't in check.".into(),
            "When you're winning, always make sure the opponent has a legal move left!".into(),
            "Material doesn't matter anymore once it's stalemate.".into(),
        ];
    }
    if pos.is_insufficient_material() {
        return vec![
            "Neither side has enough material to checkmate — it's a draw.".into(),
            "Remember: a lone king plus a bishop or knight can't force mate.".into(),
            "Try practising basic mates in the Endgames section.".into(),
        ];
    }

    // 1. Immediate tactics.
    if pos.is_check() {
        ideas.push(format!("{} is in check and must deal with it first: move the king, block, or capture the checker.", tactics::color_name(stm)));
    }
    if let Some(m) = tactics::mate_in_one(pos) {
        ideas.push(format!("{} has checkmate in one: {}!", tactics::color_name(stm), tactics::san(pos, &m)));
    } else if let Some((swapped, m)) = tactics::null_move_mate_threat(pos) {
        let mv = tactics::san(&swapped, &m);
        let back = if tactics::is_back_rank_mate(&swapped, &m) { " on the back rank" } else { "" };
        ideas.push(format!("Watch out: {} threatens {}, checkmate{} — {} must defend.", tactics::color_name(!stm), mv, back, tactics::color_name(stm)));
    }

    // 2. Hanging pieces (for the side to move these are opportunities; for the other, threats).
    let opp_hanging = tactics::hanging_pieces(b, !stm);
    if let Some(h) = opp_hanging.first() {
        let cap = tactics::best_capture_on(pos, h.square).map(|m| tactics::san(pos, &m));
        let piece = format!("{}'s {} on {}", tactics::color_name(h.color), role_name(h.role), h.square);
        ideas.push(match cap {
            Some(c) if h.undefended => format!("{} is undefended — {} can grab it with {}.", capitalize(&piece), tactics::color_name(stm), c),
            Some(c) => format!("{} is under-protected — {} wins material with {}.", capitalize(&piece), tactics::color_name(stm), c),
            None => format!("{} is loose — look for ways to attack it.", capitalize(&piece)),
        });
    }
    let own_hanging = tactics::hanging_pieces(b, stm);
    if let Some(h) = own_hanging.first() {
        ideas.push(format!(
            "{}'s {} on {} is under attack — move it, defend it, or create a bigger threat.",
            tactics::color_name(stm),
            role_name(h.role),
            h.square
        ));
    }

    // 3. Material.
    ideas.push(material_sentence(b));

    let ph = phase(pos);

    // 4. Development & king safety.
    if ph == Phase::Opening || pos.fullmoves().get() <= 14 {
        for c in [stm, !stm] {
            let undeveloped = undeveloped_minors(b, c);
            if undeveloped.len() >= 2 && pos.fullmoves().get() >= 4 {
                let names: Vec<&str> = undeveloped.iter().map(|(_, r)| role_name(*r)).collect();
                let mut uniq = names.clone();
                uniq.dedup();
                ideas.push(format!(
                    "{} still has {} minor pieces at home ({}) — developing them is a priority.",
                    side_label(c, stm),
                    undeveloped.len(),
                    uniq.iter().map(|n| format!("{n}s")).collect::<Vec<_>>().join(" and ")
                ));
                break;
            }
        }
    }
    for c in [stm, !stm] {
        let castled_rights = pos.castles().has(c, CastlingSide::KingSide) || pos.castles().has(c, CastlingSide::QueenSide);
        if king_exposed(pos, c) {
            ideas.push(format!("{}'s king looks exposed — attacking it (or tucking it away) is the key theme.", tactics::color_name(c)));
            break;
        } else if castled_rights && pos.fullmoves().get() >= 6 && ph != Phase::Endgame {
            ideas.push(format!("{} hasn't castled yet — castling soon keeps the king safe and connects the rooks.", side_label(c, stm)));
            break;
        } else if pawn_shield_weak(b, c) && ph == Phase::Middlegame {
            ideas.push(format!("The pawn cover around {}'s king is thin — keep an eye on attacks there.", tactics::color_name(c)));
            break;
        }
    }

    // 5. Passed pawns.
    for c in [stm, !stm] {
        if let Some(sq) = passed_pawns(b, c).first() {
            let advanced = match c {
                Color::White => sq.rank() >= Rank::Fifth,
                Color::Black => sq.rank() <= Rank::Fourth,
            };
            if advanced || ph == Phase::Endgame {
                ideas.push(format!(
                    "{} has a passed pawn on {} — {}",
                    tactics::color_name(c),
                    sq,
                    p.pick(61, &["push it with support, and the opponent must block it with a piece.", "passed pawns must be pushed!", "it can become a queen if nobody stops it."])
                ));
                break;
            }
        }
    }

    // 6. Endgame advice.
    if ph == Phase::Endgame {
        ideas.push(
            p.pick(62, &[
                "It's an endgame: bring the king toward the center — it's a strong piece now.",
                "Endgame time: activate your king and create a passed pawn.",
            ])
            .to_string(),
        );
    }

    // 7. Open files for rooks.
    let all_pawns = b.pawns();
    let open: Vec<File> = File::ALL.into_iter().filter(|f| (file_bb(*f) & all_pawns).is_empty()).collect();
    if !open.is_empty() && ph != Phase::Opening {
        let rooks_on: Vec<Color> = [Color::White, Color::Black]
            .into_iter()
            .filter(|c| open.iter().any(|f| !(file_bb(*f) & b.by_color(*c) & b.rooks_and_queens()).is_empty()))
            .collect();
        let names: Vec<String> = open.iter().take(2).map(|f| format!("{}-file", f.char())).collect();
        if rooks_on.len() == 1 {
            ideas.push(format!("{} controls the open {} with a heavy piece — try to invade on the 7th rank.", tactics::color_name(rooks_on[0]), names[0]));
        } else if rooks_on.is_empty() && !b.rooks().is_empty() {
            ideas.push(format!("The {} {} open — whoever puts a rook there first gains an edge.", names.join(" and "), if names.len() > 1 { "are" } else { "is" }));
        }
    }

    // 8. Pawn weaknesses.
    for c in [!stm, stm] {
        let iso = isolated_pawns(b, c);
        let dbl = doubled_files(b, c);
        if let Some(sq) = iso.first().filter(|_| ph != Phase::Opening) {
            ideas.push(format!("{}'s pawn on {} is isolated — no pawn can defend it, so it's a target.", tactics::color_name(c), sq));
            break;
        }
        if let Some(f) = dbl.first().filter(|_| ph != Phase::Opening) {
            ideas.push(format!("{} has doubled pawns on the {}-file, a small long-term weakness.", tactics::color_name(c), f.char()));
            break;
        }
    }

    // 9. Center.
    if ph == Phase::Opening {
        let center = [Square::D4, Square::E4, Square::D5, Square::E5];
        let count = |c: Color| center.iter().filter(|s| b.piece_at(**s).map(|pc| pc.color == c && pc.role == Role::Pawn).unwrap_or(false)).count();
        let (w, bl) = (count(Color::White), count(Color::Black));
        if w > bl {
            ideas.push("White has more pawns in the center — Black should challenge it with pawn breaks like ...c5 or ...d5.".into());
        } else if bl > w {
            ideas.push("Black has more central pawns — White should fight back for the center.".into());
        } else {
            ideas.push(
                p.pick(63, &[
                    "Opening principles: control the center, develop knights and bishops, and castle early.",
                    "Follow the basics: develop a new piece each move and don't move the same piece twice without a reason.",
                ])
                .to_string(),
            );
        }
    }

    // Keep 3..=6, de-duplicated.
    let mut seen = std::collections::HashSet::new();
    ideas.retain(|i| seen.insert(i.clone()));
    let fillers = [
        "Before every move, check for checks, captures and threats — for both sides.",
        "Find your least active piece and look for a better square for it.",
        "Ask yourself: what does the opponent want to do next?",
    ];
    let mut i = 0;
    while ideas.len() < 3 && i < fillers.len() {
        ideas.push(fillers[i].to_string());
        i += 1;
    }
    ideas.truncate(6);
    ideas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_position_has_ideas() {
        let ideas = describe_position("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        assert!((3..=6).contains(&ideas.len()), "{ideas:?}");
        assert!(ideas.iter().any(|i| i.contains("Material is level")));
    }

    #[test]
    fn hanging_and_material() {
        let ideas = describe_position("4k3/8/8/3n4/8/8/8/3RK3 w - - 0 1");
        assert!(ideas.iter().any(|i| i.contains("knight on d5")), "{ideas:?}");
        assert!(ideas.iter().any(|i| i.contains("up")), "{ideas:?}");
    }

    #[test]
    fn mate_threat_noticed() {
        let ideas = describe_position("6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1");
        assert!(ideas.iter().any(|i| i.contains("Ra8#")), "{ideas:?}");
    }

    #[test]
    fn invalid_fen() {
        assert_eq!(describe_position("garbage").len(), 1);
    }
}
