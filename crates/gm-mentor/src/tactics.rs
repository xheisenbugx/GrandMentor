//! Board-level motif detection used by the coach: static exchange evaluation, hanging pieces,
//! forks, pins, skewers, mate-in-one threats and a bundle of facts about a single move.
//!
//! Everything here is pure, allocation-light and panic-free; it never searches deeper than one
//! ply (plus a mate-in-one probe), so it is cheap enough to run for every move of a review.

use shakmaty::san::SanPlus;
use shakmaty::{attacks, Bitboard, Board, CastlingSide, Chess, Color, Move, Position, Role, Square};

/// Piece values in centipawns (king is "infinite" for exchange purposes).
pub fn value(role: Role) -> i32 {
    match role {
        Role::Pawn => 100,
        Role::Knight => 300,
        Role::Bishop => 320,
        Role::Rook => 500,
        Role::Queen => 900,
        Role::King => 20_000,
    }
}

/// Material points used when talking to humans (1/3/3/5/9).
pub fn points(role: Role) -> i32 {
    match role {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 0,
    }
}

pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::Pawn => "pawn",
        Role::Knight => "knight",
        Role::Bishop => "bishop",
        Role::Rook => "rook",
        Role::Queen => "queen",
        Role::King => "king",
    }
}

pub fn color_name(c: Color) -> &'static str {
    match c {
        Color::White => "White",
        Color::Black => "Black",
    }
}

/// SAN with check/mate suffix.
pub fn san(pos: &Chess, m: &Move) -> String {
    SanPlus::from_move(pos.clone(), m).to_string()
}

/// Parse a SAN move (tolerant of `+`, `#`, `!`, `?` suffixes and `0-0` castling).
pub fn parse_san(pos: &Chess, text: &str) -> Option<Move> {
    let t = text.trim().trim_end_matches(['!', '?']);
    if t.is_empty() || t.len() > 12 {
        return None;
    }
    let t = t.replace('0', "O");
    let sp = SanPlus::from_ascii(t.as_bytes()).ok()?;
    sp.san.to_move(pos).ok()
}

/// Parse a move given either as UCI or SAN.
pub fn parse_any_move(pos: &Chess, text: &str) -> Option<Move> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(m) = gm_engine::uci_to_move(pos, t) {
        return Some(m);
    }
    parse_san(pos, t)
}

/// Total material in points for one side.
pub fn material_points(board: &Board, c: Color) -> i32 {
    let mut total = 0;
    for role in [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen] {
        total += points(role) * (board.by_color(c) & board.by_role(role)).count() as i32;
    }
    total
}

/// Non-pawn material (centipawns) for one side, kings excluded.
pub fn piece_material(board: &Board, c: Color) -> i32 {
    let mut total = 0;
    for role in [Role::Knight, Role::Bishop, Role::Rook, Role::Queen] {
        total += value(role) * (board.by_color(c) & board.by_role(role)).count() as i32;
    }
    total
}

fn least_valuable_attacker(board: &Board, sq: Square, side: Color, occ: Bitboard) -> Option<(Square, i32)> {
    let atk = board.attacks_to(sq, side, occ) & occ;
    if atk.is_empty() {
        return None;
    }
    for role in [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen, Role::King] {
        if let Some(s) = (atk & board.by_role(role)).first() {
            return Some((s, value(role)));
        }
    }
    None
}

/// Static exchange evaluation: material `side` gains by initiating captures on `sq`
/// (occupied by an enemy piece). Returns 0 when the square is empty or unattacked.
/// Pins are ignored, x-rays are handled.
pub fn see(board: &Board, sq: Square, side: Color) -> i32 {
    let Some(target) = board.role_at(sq) else { return 0 };
    if board.color_at(sq) == Some(side) {
        return 0;
    }
    let mut occ = board.occupied();
    let Some(mut attacker) = least_valuable_attacker(board, sq, side, occ) else { return 0 };
    let mut gain = [0i32; 34];
    let mut d = 0usize;
    gain[0] = value(target);
    let mut stm = side;
    loop {
        d += 1;
        gain[d] = attacker.1 - gain[d - 1];
        occ.discard(attacker.0);
        stm = !stm;
        match least_valuable_attacker(board, sq, stm, occ) {
            Some(next) => attacker = next,
            None => break,
        }
        if d >= 32 {
            break;
        }
    }
    while d > 1 {
        d -= 1;
        gain[d - 1] = -(-gain[d - 1]).max(gain[d]);
    }
    gain[0]
}

/// A piece the opponent can win.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hanging {
    pub square: Square,
    pub role: Role,
    pub color: Color,
    /// Centipawns the opponent wins by capturing (SEE).
    pub gain: i32,
    /// True when no piece defends it at all.
    pub undefended: bool,
}

/// Pieces of `color` (kings excluded) that the opponent can win by exchange, biggest first.
pub fn hanging_pieces(board: &Board, color: Color) -> Vec<Hanging> {
    let mut out = Vec::new();
    let occ = board.occupied();
    for sq in board.by_color(color) & !board.kings() {
        let gain = see(board, sq, !color);
        if gain >= 90 {
            if let Some(role) = board.role_at(sq) {
                let undefended = (board.attacks_to(sq, color, occ) & occ).is_empty();
                out.push(Hanging { square: sq, role, color, gain, undefended });
            }
        }
    }
    out.sort_by(|a, b| b.gain.cmp(&a.gain).then(value(b.role).cmp(&value(a.role))));
    out
}

/// The legal capture on `sq` using the least valuable piece (side to move).
pub fn best_capture_on(pos: &Chess, sq: Square) -> Option<Move> {
    pos.legal_moves()
        .into_iter()
        .filter(|m| m.to() == sq && m.is_capture())
        .min_by_key(|m| value(m.role()))
}

/// A checkmating move for the side to move, if any.
pub fn mate_in_one(pos: &Chess) -> Option<Move> {
    if pos.is_game_over() {
        return None;
    }
    let moves = pos.legal_moves();
    // Checks first is just an ordering nicety; every mate is a check anyway.
    moves.into_iter().find(|m| {
        let mut p = pos.clone();
        p.play_unchecked(m);
        p.is_checkmate()
    })
}

/// What the side NOT to move would threaten if it were its turn (null-move probe):
/// a mate in one. Returns None if the position can't be turned (e.g. side to move in check).
pub fn null_move_mate_threat(pos: &Chess) -> Option<(Chess, Move)> {
    if pos.is_check() {
        return None;
    }
    let swapped = pos.clone().swap_turn().ok()?;
    let m = mate_in_one(&swapped)?;
    Some((swapped, m))
}

/// Whether a mate lands on the defender's back rank with a rook or queen (classic back-rank mate).
pub fn is_back_rank_mate(pos_before: &Chess, m: &Move) -> bool {
    let attacker = pos_before.turn();
    let back = match attacker {
        Color::White => shakmaty::Rank::Eighth,
        Color::Black => shakmaty::Rank::First,
    };
    matches!(m.role(), Role::Rook | Role::Queen)
        && m.to().rank() == back
        && pos_before.board().king_of(!attacker).map(|k| k.rank() == back).unwrap_or(false)
}

/// Pieces attacked by the piece standing on `from` that are "fork-worthy": the king, anything
/// more valuable than the attacker, or an undefended non-pawn.
pub fn fork_targets(board: &Board, from: Square) -> Vec<(Square, Role)> {
    let Some(piece) = board.piece_at(from) else { return Vec::new() };
    let occ = board.occupied();
    let mut out = Vec::new();
    for t in board.attacks_from(from) & board.by_color(!piece.color) {
        let Some(role) = board.role_at(t) else { continue };
        let undefended = (board.attacks_to(t, !piece.color, occ) & occ).is_empty();
        if role == Role::King || value(role) > value(piece.role) || (undefended && role != Role::Pawn) {
            out.push((t, role));
        }
    }
    out.sort_by_key(|(_, r)| -value(*r));
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineKind {
    Pin,
    Skewer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineTactic {
    pub kind: LineKind,
    pub front: (Square, Role),
    pub behind: (Square, Role),
}

/// Pins and skewers created by the slider on `from`.
pub fn line_tactics(board: &Board, from: Square) -> Vec<LineTactic> {
    let Some(piece) = board.piece_at(from) else { return Vec::new() };
    if !matches!(piece.role, Role::Bishop | Role::Rook | Role::Queen) {
        return Vec::new();
    }
    let occ = board.occupied();
    let them = board.by_color(!piece.color);
    let mut out = Vec::new();
    for t in board.attacks_from(from) & them {
        let line = attacks::ray(from, t);
        let mut behind = None;
        for s in line & occ {
            if s == t || s == from {
                continue;
            }
            if attacks::between(from, s).contains(t) && (attacks::between(t, s) & occ).is_empty() {
                behind = Some(s);
                break;
            }
        }
        let Some(b) = behind else { continue };
        if board.color_at(b) != Some(!piece.color) {
            continue;
        }
        let (Some(fr), Some(br)) = (board.role_at(t), board.role_at(b)) else { continue };
        let behind_undefended = (board.attacks_to(b, !piece.color, occ) & occ).is_empty();
        if br == Role::King && fr != Role::King {
            out.push(LineTactic { kind: LineKind::Pin, front: (t, fr), behind: (b, br) });
        } else if br != Role::Pawn
            && (behind_undefended || value(br) >= value(piece.role))
            && (fr == Role::King || (value(fr) > value(br) && value(fr) >= value(Role::Rook)))
        {
            out.push(LineTactic { kind: LineKind::Skewer, front: (t, fr), behind: (b, br) });
        } else if value(br) > value(fr) + 50 && value(br) > value(piece.role) && fr != Role::Pawn {
            out.push(LineTactic { kind: LineKind::Pin, front: (t, fr), behind: (b, br) });
        }
    }
    out
}

const CENTER: [Square; 4] = [Square::D4, Square::E4, Square::D5, Square::E5];

fn center_bb() -> Bitboard {
    let mut bb = Bitboard::EMPTY;
    for s in CENTER {
        bb.add(s);
    }
    bb
}

/// Everything the coach wants to know about one move.
#[derive(Clone, Debug)]
pub struct MoveFacts {
    pub san: String,
    pub mover: Color,
    pub role: Role,
    pub from: Option<Square>,
    pub to: Square,
    pub captured: Option<Role>,
    /// Material outcome of the move for the mover once the exchange on `to` settles (cp).
    pub capture_net: i32,
    /// Whether the opponent can take back on `to` at all.
    pub recapturable: bool,
    pub gives_check: bool,
    pub is_mate: bool,
    pub is_stalemate: bool,
    pub castle: Option<CastlingSide>,
    pub promotion: Option<Role>,
    pub develops: bool,
    pub center: bool,
    pub fork: Vec<(Square, Role)>,
    pub lines: Vec<LineTactic>,
    /// Mover's pieces the opponent can win after the move (biggest first), with the winning capture SAN.
    pub hangs: Vec<(Hanging, Option<String>, bool /* was already hanging before */)>,
    /// Opponent's checkmate reply, if the move allows one.
    pub allows_mate: Option<(String, bool /* back rank */)>,
    /// Mover's mate-in-one threat for next turn.
    pub threatens_mate: Option<String>,
    /// Mover had a mate in one and this move isn't it.
    pub missed_mate: Option<String>,
    /// The moved piece was hanging before and isn't after.
    pub saves_piece: bool,
    /// The opponent threatened this mate before the move; the move stops it.
    pub stops_mate: Option<String>,
    /// Another of the mover's pieces that was hanging before and is safe after.
    pub defends: Option<(Square, Role)>,
    /// Newly attacked enemy piece that can now be won (first one).
    pub new_threat: Option<(Square, Role)>,
    pub king_walk: bool,
    pub weakens_king: bool,
    pub opening_phase: bool,
    /// Mover's material lead in points before the move.
    pub material_lead: i32,
}

impl MoveFacts {
    pub fn is_trade(&self) -> bool {
        self.captured.is_some() && self.recapturable && self.capture_net.abs() <= 60
    }
    pub fn wins_material(&self) -> bool {
        self.capture_net >= 90
    }
}

fn is_back_rank(c: Color, sq: Square) -> bool {
    match c {
        Color::White => sq.rank() == shakmaty::Rank::First,
        Color::Black => sq.rank() == shakmaty::Rank::Eighth,
    }
}

/// Compute facts for `m` played in `pos` (must be legal in `pos`).
pub fn move_facts(pos: &Chess, m: &Move) -> MoveFacts {
    let mover = pos.turn();
    let board_before = pos.board();
    let mut after = pos.clone();
    after.play_unchecked(m);
    let board = after.board();
    let to = m.to();
    let from = m.from();
    let role = m.role();
    let san = san(pos, m);
    let fullmoves = pos.fullmoves().get();
    let opening_phase = fullmoves <= 12;

    let captured = m.capture();
    let castle = m.castling_side();
    // Where the moving piece ends up (castling `to` is the rook square in shakmaty's encoding).
    let landed = match castle {
        Some(side) => Square::from_coords(side.king_to_file(), to.rank()),
        None => to,
    };
    let rook_landed = castle.map(|side| Square::from_coords(side.rook_to_file(), to.rank()));

    let is_mate = after.is_checkmate();
    let opp_win_on_to = if castle.is_some() { 0 } else { see(board, landed, !mover).max(0) };
    let recapturable = castle.is_none() && !(board.attacks_to(landed, !mover, board.occupied())).is_empty();
    let capture_net = captured.map(value).unwrap_or(0) - if is_mate { 0 } else { opp_win_on_to };

    // Hanging pieces after the move.
    let before_hanging: Vec<Hanging> = hanging_pieces(board_before, mover);
    let mut hangs = Vec::new();
    if !is_mate {
        for h in hanging_pieces(board, mover) {
            if h.square == landed && captured.is_some() {
                // Covered by capture_net (an exchange that the mover started).
                continue;
            }
            let was = before_hanging.iter().any(|b| b.square == h.square) && h.square != landed;
            let cap = best_capture_on(&after, h.square).map(|cm| self::san(&after, &cm));
            hangs.push((h, cap, was));
        }
    }

    let allows_mate = if is_mate {
        None
    } else {
        mate_in_one(&after).map(|mm| (self::san(&after, &mm), is_back_rank_mate(&after, &mm)))
    };
    let threatens_mate = if is_mate || after.is_check() {
        None
    } else {
        null_move_mate_threat(&after).map(|(p, mm)| self::san(&p, &mm))
    };
    let missed_mate = if is_mate { None } else { mate_in_one(pos).map(|mm| self::san(pos, &mm)) };

    let saves_piece = castle.is_none()
        && from.map(|f| before_hanging.iter().any(|h| h.square == f)).unwrap_or(false)
        && see(board, landed, !mover) < 90;

    let stops_mate = if allows_mate.is_none() {
        null_move_mate_threat(pos).map(|(p, mm)| self::san(&p, &mm))
    } else {
        None
    };
    let defends = if is_mate {
        None
    } else {
        let after_hanging = hanging_pieces(board, mover);
        before_hanging
            .iter()
            .find(|h| Some(h.square) != from && board.piece_at(h.square).map(|pc| pc.color == mover).unwrap_or(false)
                && !after_hanging.iter().any(|a| a.square == h.square))
            .map(|h| (h.square, h.role))
    };

    // New threats by the moved piece.
    let opp_before: Vec<Square> = hanging_pieces(board_before, !mover).iter().map(|h| h.square).collect();
    let mut new_threat = None;
    if !is_mate {
        for h in hanging_pieces(board, !mover) {
            if !opp_before.contains(&h.square) && board.attacks_from(landed).contains(h.square) {
                new_threat = Some((h.square, h.role));
                break;
            }
        }
    }

    let fork = if castle.is_some() || is_mate {
        Vec::new()
    } else {
        let targets = fork_targets(board, landed);
        let safe = see(board, landed, !mover) <= 0 || targets.iter().any(|(_, r)| *r == Role::King);
        if targets.len() >= 2 && safe {
            targets
        } else {
            Vec::new()
        }
    };
    let mut lines = if is_mate { Vec::new() } else { line_tactics(board, landed) };
    if let Some(r) = rook_landed {
        lines.extend(line_tactics(board, r));
    }

    let develops = castle.is_none()
        && matches!(role, Role::Knight | Role::Bishop)
        && from.map(|f| is_back_rank(mover, f)).unwrap_or(false)
        && fullmoves <= 15;
    let center = castle.is_none()
        && role != Role::King
        && (CENTER.contains(&landed)
            || (board.attacks_from(landed) & center_bb()).count() >= 2 && !matches!(role, Role::Queen | Role::King) && opening_phase);

    let queens_on = !board_before.queens().is_empty();
    let king_walk = role == Role::King && castle.is_none() && fullmoves <= 20 && queens_on
        && piece_material(board_before, !mover) >= 1500;
    let weakens_king = role == Role::Pawn && queens_on && {
        if let Some(k) = board_before.king_of(mover) {
            let kf = k.file() as i32;
            let pf = to.file() as i32;
            let castled_side = k.file() >= shakmaty::File::G || k.file() <= shakmaty::File::C;
            castled_side && (kf - pf).abs() <= 1 && is_back_rank(mover, k) && from.map(|f| f.distance(to) >= 1).unwrap_or(false)
                && captured.is_none()
        } else {
            false
        }
    };

    let material_lead = material_points(board_before, mover) - material_points(board_before, !mover);

    MoveFacts {
        san,
        mover,
        role,
        from,
        to: landed,
        captured,
        capture_net,
        recapturable,
        gives_check: after.is_check(),
        is_mate,
        is_stalemate: after.is_stalemate(),
        castle,
        promotion: m.promotion(),
        develops,
        center,
        fork,
        lines,
        hangs,
        allows_mate,
        threatens_mate,
        missed_mate,
        saves_piece,
        stops_mate,
        defends,
        new_threat,
        king_walk,
        weakens_king,
        opening_phase,
        material_lead,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(fen: &str) -> Chess {
        gm_engine::parse_fen(fen).expect("fen")
    }

    #[test]
    fn see_basic() {
        // White pawn e4 can take undefended knight d5.
        let p = pos("4k3/8/8/3n4/4P3/8/8/4K3 w - - 0 1");
        assert_eq!(see(p.board(), Square::D5, Color::White), 300);
        // Defended by a pawn: P x N, p x P => 200.
        let p = pos("4k3/8/4p3/3n4/4P3/8/8/4K3 w - - 0 1");
        assert_eq!(see(p.board(), Square::D5, Color::White), 200);
        // Queen takes pawn defended by pawn: loses.
        let p = pos("4k3/8/2p5/3p4/8/8/8/3QK3 w - - 0 1");
        assert!(see(p.board(), Square::D5, Color::White) < 0);
    }

    #[test]
    fn hanging_detects_undefended_piece() {
        let p = pos("4k3/8/8/3n4/8/8/8/3RK3 w - - 0 1");
        let h = hanging_pieces(p.board(), Color::Black);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].role, Role::Knight);
    }

    #[test]
    fn fork_and_pin() {
        // Knight on c7 forks king e8 and rook a8.
        let p = pos("r3k3/2N5/8/8/8/8/8/4K3 b - - 0 1");
        assert_eq!(fork_targets(p.board(), Square::C7).len(), 2);
        // Bishop b5 pins knight c6 to king e8.
        let p = pos("4k3/8/2n5/1B6/8/8/8/4K3 b - - 0 1");
        let l = line_tactics(p.board(), Square::B5);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].kind, LineKind::Pin);
    }

    #[test]
    fn mate_in_one_found() {
        let p = pos("6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1");
        let m = mate_in_one(&p).expect("mate");
        assert_eq!(san(&p, &m), "Ra8#");
        assert!(is_back_rank_mate(&p, &m));
    }
}
