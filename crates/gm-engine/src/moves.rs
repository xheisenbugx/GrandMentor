//! Move helpers for the search: compact encoding, incremental Zobrist hashing, SEE, MVV-LVA.

use shakmaty::zobrist::{Zobrist64, ZobristHash, ZobristValue};
use shakmaty::{
    Board, CastlingSide, Chess, Color, EnPassantMode, Move, Piece, Position, Role, Square,
};

/// SEE / ordering piece values indexed by `Role as usize` (index 0 unused).
pub const SEE_VALUE: [i32; 7] = [0, 100, 320, 330, 500, 950, 20_000];

#[inline]
pub fn value(role: Role) -> i32 {
    SEE_VALUE[role as usize]
}

/// Compact 16-bit move code: from(6) | to(6) << 6 | promotion role(3) << 12. 0 = no move.
/// Castling is encoded king-square -> rook-square (shakmaty's internal representation).
#[inline]
pub fn encode(m: &Move) -> u16 {
    let from = m.from().map(u32::from).unwrap_or(0) as u16;
    let to = u32::from(m.to()) as u16;
    let promo = m.promotion().map(|r| r as u16).unwrap_or(0);
    from | (to << 6) | (promo << 12)
}

/// Full Zobrist hash (matches shakmaty's `zobrist_hash` with legal en passant).
#[inline]
pub fn full_hash(pos: &Chess) -> u64 {
    pos.zobrist_hash::<Zobrist64>(EnPassantMode::Legal).0
}

#[inline]
fn zp(sq: Square, color: Color, role: Role) -> u64 {
    Zobrist64::zobrist_for_piece(sq, Piece { color, role }).0
}

#[inline]
fn ep_key(pos: &Chess) -> u64 {
    match pos.maybe_ep_square() {
        None => 0,
        Some(_) => match pos.ep_square(EnPassantMode::Legal) {
            Some(sq) => Zobrist64::zobrist_for_en_passant_file(sq.file()).0,
            None => 0,
        },
    }
}

/// Hash of `after` (= `before` + `m`) computed incrementally from the hash of `before`.
pub fn hash_after(h: u64, before: &Chess, m: &Move, after: &Chess) -> u64 {
    let us = before.turn();
    let them = !us;
    let mut h = h ^ Zobrist64::zobrist_for_white_turn().0;
    match *m {
        Move::Normal {
            role,
            from,
            capture,
            to,
            promotion,
        } => {
            h ^= zp(from, us, role) ^ zp(to, us, promotion.unwrap_or(role));
            if let Some(c) = capture {
                h ^= zp(to, them, c);
            }
        }
        Move::EnPassant { from, to } => {
            h ^= zp(from, us, Role::Pawn) ^ zp(to, us, Role::Pawn);
            h ^= zp(
                Square::from_coords(to.file(), from.rank()),
                them,
                Role::Pawn,
            );
        }
        Move::Castle { king, rook } => {
            let side = if rook > king {
                CastlingSide::KingSide
            } else {
                CastlingSide::QueenSide
            };
            h ^= zp(king, us, Role::King) ^ zp(side.king_to(us), us, Role::King);
            h ^= zp(rook, us, Role::Rook) ^ zp(side.rook_to(us), us, Role::Rook);
        }
        Move::Put { .. } => return full_hash(after),
    }
    let cb = before.castles();
    let ca = after.castles();
    for color in [Color::White, Color::Black] {
        for side in [CastlingSide::KingSide, CastlingSide::QueenSide] {
            if cb.has(color, side) != ca.has(color, side) {
                h ^= Zobrist64::zobrist_for_castling_right(color, side).0;
            }
        }
    }
    h ^ ep_key(before) ^ ep_key(after)
}

/// Hash after a null move (side flips, en passant rights vanish).
pub fn hash_null(h: u64, before: &Chess) -> u64 {
    h ^ Zobrist64::zobrist_for_white_turn().0 ^ ep_key(before)
}

/// Least valuable attacker of `side` among `attackers`.
#[inline]
fn least_valuable(
    board: &Board,
    attackers: shakmaty::Bitboard,
    side: Color,
) -> Option<(Square, Role)> {
    let ours = attackers & board.by_color(side);
    if ours.is_empty() {
        return None;
    }
    for role in Role::ALL {
        if let Some(sq) = (ours & board.by_role(role)).first() {
            return Some((sq, role));
        }
    }
    None
}

/// Static exchange evaluation of a move (material balance for the mover, in SEE_VALUE units).
pub fn see(pos: &Chess, m: &Move) -> i32 {
    let (from, to, captured, mover) = match *m {
        Move::Normal {
            role,
            from,
            capture,
            to,
            promotion,
        } => {
            let cap = capture.map(value).unwrap_or(0);
            let promo_gain = promotion.map(|p| value(p) - value(Role::Pawn)).unwrap_or(0);
            let mover = promotion.unwrap_or(role);
            (from, to, cap + promo_gain, mover)
        }
        Move::EnPassant { .. } => return 0,
        Move::Castle { .. } | Move::Put { .. } => return 0,
    };
    let board = pos.board();
    let mut occ = board.occupied();
    occ.discard(from);
    let mut gain = [0i32; 34];
    gain[0] = captured;
    let mut d = 0usize;
    let mut side = !pos.turn();
    let mut attacker_value = value(mover);
    loop {
        d += 1;
        if d >= gain.len() {
            break;
        }
        gain[d] = attacker_value - gain[d - 1];
        let attackers = (board.attacks_to(to, Color::White, occ)
            | board.attacks_to(to, Color::Black, occ))
            & occ;
        let Some((sq, role)) = least_valuable(board, attackers, side) else {
            break;
        };
        if role == Role::King && (attackers & board.by_color(!side)).any() {
            // the king cannot capture into a defended square
            break;
        }
        occ.discard(sq);
        attacker_value = value(role);
        side = !side;
    }
    while d > 1 {
        d -= 1;
        gain[d - 1] = -(-gain[d - 1]).max(gain[d]);
    }
    gain[0]
}

/// Does `see(pos, m) >= threshold`?
#[inline]
pub fn see_ge(pos: &Chess, m: &Move, threshold: i32) -> bool {
    see(pos, m) >= threshold
}

/// MVV-LVA ordering key for captures.
#[inline]
pub fn mvv_lva(m: &Move) -> i32 {
    let victim = match m {
        Move::EnPassant { .. } => value(Role::Pawn),
        _ => m.capture().map(value).unwrap_or(0),
    };
    let promo = m.promotion().map(value).unwrap_or(0);
    let attacker = match m.role() {
        Role::King => 0,
        r => value(r),
    };
    (victim + promo) * 16 - attacker / 16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_fen, uci_to_move};

    #[test]
    fn incremental_hash_matches_full() {
        let fens = [
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        ];
        for f in fens {
            let p = parse_fen(f).unwrap();
            let h = full_hash(&p);
            for m in p.legal_moves() {
                let mut c = p.clone();
                c.play_unchecked(&m);
                let hc = full_hash(&c);
                assert_eq!(hash_after(h, &p, &m, &c), hc, "{f} {m:?}");
                for m2 in c.legal_moves() {
                    let mut c2 = c.clone();
                    c2.play_unchecked(&m2);
                    assert_eq!(
                        hash_after(hc, &c, &m2, &c2),
                        full_hash(&c2),
                        "{f} {m:?} {m2:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn see_values() {
        // pawn takes defended knight: wins knight for pawn
        let p = parse_fen("4k3/2p5/3n4/4P3/8/8/8/4K3 w - - 0 1").unwrap();
        let m = uci_to_move(&p, "e5d6").unwrap();
        assert_eq!(see(&p, &m), 220); // NxP... PxN, then cxd6 recaptures
                                      // queen takes pawn defended by pawn: loses queen
        let p = parse_fen("4k3/8/2p5/3p4/8/8/3Q4/4K3 w - - 0 1").unwrap();
        let m = uci_to_move(&p, "d2d5").unwrap();
        assert!(see(&p, &m) < 0);
        // rook takes undefended rook
        let p = parse_fen("4k3/8/8/3r4/8/8/3R4/4K3 w - - 0 1").unwrap();
        let m = uci_to_move(&p, "d2d5").unwrap();
        assert_eq!(see(&p, &m), 500);
    }
}
