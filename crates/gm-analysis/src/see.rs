//! Static exchange evaluation (SEE) and material helpers used to detect sacrifices.

use shakmaty::{Board, Chess, Color, Move, Position, Role, Square};

/// Conventional piece values in centipawns. The king is "infinite" so exchanges never
/// profitably end with a king capture into a defended square.
pub(crate) fn piece_value(role: Role) -> i32 {
    match role {
        Role::Pawn => 100,
        Role::Knight => 300,
        Role::Bishop => 310,
        Role::Rook => 500,
        Role::Queen => 900,
        Role::King => 20_000,
    }
}

const SWAP_ORDER: [Role; 6] = [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen, Role::King];

/// Static exchange evaluation of `m` (a capture) on `board`, from the capturing side's point
/// of view, in centipawns. Handles x-ray attackers through the shrinking occupancy. Pins are
/// ignored beyond the first capture (which is assumed legal).
pub(crate) fn see(board: &Board, m: &Move) -> i32 {
    let Some(from) = m.from() else { return 0 };
    let to: Square = m.to();
    let Some(captured) = m.capture() else { return 0 };
    let Some(mover) = board.color_at(from) else { return 0 };

    let mut occupied = board.occupied();
    occupied.discard(from);
    if m.is_en_passant() {
        // The captured pawn is not on `to`; remove it from the occupancy.
        let ep_sq = Square::from_coords(to.file(), from.rank());
        occupied.discard(ep_sq);
    }

    let mut gains = [0i32; 34];
    gains[0] = piece_value(captured);
    let mut last_value = match m.promotion() {
        Some(p) => piece_value(p),
        None => piece_value(m.role()),
    };
    let mut side = mover.other();
    let mut depth = 0usize;

    while depth + 1 < gains.len() {
        let attackers = board.attacks_to(to, side, occupied) & occupied;
        if attackers.is_empty() {
            break;
        }
        let Some((sq, role)) = SWAP_ORDER
            .iter()
            .find_map(|&r| (attackers & board.by_role(r)).first().map(|sq| (sq, r)))
        else {
            break;
        };
        depth += 1;
        gains[depth] = last_value - gains[depth - 1];
        // Neither side can improve on this exchange: stop early (standard pruning).
        if gains[depth].max(-gains[depth - 1]) < 0 {
            break;
        }
        last_value = piece_value(role);
        occupied.discard(sq);
        side = side.other();
    }
    while depth > 0 {
        gains[depth - 1] = -((-gains[depth - 1]).max(gains[depth]));
        depth -= 1;
    }
    gains[0]
}

/// The most material the side to move in `pos` can win immediately by a capture,
/// according to SEE (0 if nothing is en prise).
pub(crate) fn max_capture_gain(pos: &Chess) -> i32 {
    let board = pos.board();
    pos.legal_moves()
        .iter()
        .filter(|m| m.is_capture())
        .map(|m| see(board, m))
        .max()
        .unwrap_or(0)
        .max(0)
}

/// Material immediately gained by playing `m` (captured piece + promotion upgrade).
pub(crate) fn immediate_gain(m: &Move) -> i32 {
    let captured = m.capture().map(piece_value).unwrap_or(0);
    let promo = m.promotion().map(|p| piece_value(p) - piece_value(Role::Pawn)).unwrap_or(0);
    captured + promo
}

/// Material balance (white minus black) without kings, centipawns.
#[allow(dead_code)]
pub(crate) fn material_balance(pos: &Chess) -> i32 {
    let board = pos.board();
    let mut total = 0;
    for role in [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen] {
        let bb = board.by_role(role);
        let w = (bb & board.by_color(Color::White)).count() as i32;
        let b = (bb & board.by_color(Color::Black)).count() as i32;
        total += (w - b) * piece_value(role);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use gm_engine::{parse_fen, uci_to_move};

    fn see_of(fen: &str, uci: &str) -> i32 {
        let pos = parse_fen(fen).expect("fen");
        let m = uci_to_move(&pos, uci).expect("move");
        see(pos.board(), &m)
    }

    #[test]
    fn free_piece() {
        // White rook takes an undefended knight.
        assert_eq!(see_of("4k3/8/8/3n4/8/8/8/3RK3 w - - 0 1", "d1d5"), 300);
    }

    #[test]
    fn defended_pawn_by_queen_loses() {
        // Queen takes a pawn defended by a pawn: -800.
        assert_eq!(see_of("4k3/8/2p5/3p4/8/8/8/3QK3 w - - 0 1", "d1d5"), 100 - 900);
    }

    #[test]
    fn xray_battery() {
        // Rook takes pawn defended by rook, backed up by a second rook: wins the pawn.
        // d5 pawn defended by black rook d8; white rooks d1,d2.
        assert_eq!(see_of("3rk3/8/8/3p4/8/8/3R4/3RK3 w - - 0 1", "d2d5"), 100);
    }

    #[test]
    fn max_gain_detects_hanging_queen() {
        let pos = parse_fen("4k3/8/8/3q4/8/8/8/3RK3 w - - 0 1").expect("fen");
        assert_eq!(max_capture_gain(&pos), 900);
    }
}
