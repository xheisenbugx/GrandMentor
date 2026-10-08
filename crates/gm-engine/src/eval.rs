//! Static evaluation: tapered PeSTO piece-square tables plus mobility, king safety,
//! pawn structure, rook files, bishop pair, endgame mop-up and drawish-material scaling.
//!
//! All terms are accumulated per colour as (middlegame, endgame) pairs and blended by game
//! phase. The public result is from the side-to-move's point of view, in centipawns.

use shakmaty::attacks::{
    bishop_attacks, king_attacks, knight_attacks, queen_attacks, rook_attacks,
};
use shakmaty::{Bitboard, Board, Chess, Color, Piece, Position, Role, Square};

/// Maximum absolute value `evaluate` can return (keeps eval well below mate scores).
pub const EVAL_LIMIT: i32 = 20_000;

// ---------------------------------------------------------------------------------------------
// PeSTO tables (Rofchade / CPW). Tables are laid out visually: index 0 = a8 ... 63 = h1.
// ---------------------------------------------------------------------------------------------

const MG_VALUE: [i32; 6] = [82, 337, 365, 477, 1025, 0];
const EG_VALUE: [i32; 6] = [94, 281, 297, 512, 936, 0];
const PHASE_INC: [i32; 6] = [0, 1, 1, 2, 4, 0];

#[rustfmt::skip]
const MG_PST: [[i32; 64]; 6] = [
    // pawn
    [  0,   0,   0,   0,   0,   0,  0,   0,
      98, 134,  61,  95,  68, 126, 34, -11,
      -6,   7,  26,  31,  65,  56, 25, -20,
     -14,  13,   6,  21,  23,  12, 17, -23,
     -27,  -2,  -5,  12,  17,   6, 10, -25,
     -26,  -4,  -4, -10,   3,   3, 33, -12,
     -35,  -1, -20, -23, -15,  24, 38, -22,
       0,   0,   0,   0,   0,   0,  0,   0],
    // knight
    [-167, -89, -34, -49,  61, -97, -15, -107,
      -73, -41,  72,  36,  23,  62,   7,  -17,
      -47,  60,  37,  65,  84, 129,  73,   44,
       -9,  17,  19,  53,  37,  69,  18,   22,
      -13,   4,  16,  13,  28,  19,  21,   -8,
      -23,  -9,  12,  10,  19,  17,  25,  -16,
      -29, -53, -12,  -3,  -1,  18, -14,  -19,
     -105, -21, -58, -33, -17, -28, -19,  -23],
    // bishop
    [-29,   4, -82, -37, -25, -42,   7,  -8,
     -26,  16, -18, -13,  30,  59,  18, -47,
     -16,  37,  43,  40,  35,  50,  37,  -2,
      -4,   5,  19,  50,  37,  37,   7,  -2,
      -6,  13,  13,  26,  34,  12,  10,   4,
       0,  15,  15,  15,  14,  27,  18,  10,
       4,  15,  16,   0,   7,  21,  33,   1,
     -33,  -3, -14, -21, -13, -12, -39, -21],
    // rook
    [ 32,  42,  32,  51, 63,  9,  31,  43,
      27,  32,  58,  62, 80, 67,  26,  44,
      -5,  19,  26,  36, 17, 45,  61,  16,
     -24, -11,   7,  26, 24, 35,  -8, -20,
     -36, -26, -12,  -1,  9, -7,   6, -23,
     -45, -25, -16, -17,  3,  0,  -5, -33,
     -44, -16, -20,  -9, -1, 11,  -6, -71,
     -19, -13,   1,  17, 16,  7, -37, -26],
    // queen
    [-28,   0,  29,  12,  59,  44,  43,  45,
     -24, -39,  -5,   1, -16,  57,  28,  54,
     -13, -17,   7,   8,  29,  56,  47,  57,
     -27, -27, -16, -16,  -1,  17,  -2,   1,
      -9, -26,  -9, -10,  -2,  -4,   3,  -3,
     -14,   2, -11,  -2,  -5,   2,  14,   5,
     -35,  -8,  11,   2,   8,  15,  -3,   1,
      -1, -18,  -9,  10, -15, -25, -31, -50],
    // king
    [-65,  23,  16, -15, -56, -34,   2,  13,
      29,  -1, -20,  -7,  -8,  -4, -38, -29,
      -9,  24,   2, -16, -20,   6,  22, -22,
     -17, -20, -12, -27, -30, -25, -14, -36,
     -49,  -1, -27, -39, -46, -44, -33, -51,
     -14, -14, -22, -46, -44, -30, -15, -27,
       1,   7,  -8, -64, -43, -16,   9,   8,
     -15,  36,  12, -54,   8, -28,  24,  14],
];

#[rustfmt::skip]
const EG_PST: [[i32; 64]; 6] = [
    // pawn
    [  0,   0,   0,   0,   0,   0,   0,   0,
     178, 173, 158, 134, 147, 132, 165, 187,
      94, 100,  85,  67,  56,  53,  82,  84,
      32,  24,  13,   5,  -2,   4,  17,  17,
      13,   9,  -3,  -7,  -7,  -8,   3,  -1,
       4,   7,  -6,   1,   0,  -5,  -1,  -8,
      13,   8,   8,  10,  13,   0,   2,  -7,
       0,   0,   0,   0,   0,   0,   0,   0],
    // knight
    [-58, -38, -13, -28, -31, -27, -63, -99,
     -25,  -8, -25,  -2,  -9, -25, -24, -52,
     -24, -20,  10,   9,  -1,  -9, -19, -41,
     -17,   3,  22,  22,  22,  11,   8, -18,
     -18,  -6,  16,  25,  16,  17,   4, -18,
     -23,  -3,  -1,  15,  10,  -3, -20, -22,
     -42, -20, -10,  -5,  -2, -20, -23, -44,
     -29, -51, -23, -15, -22, -18, -50, -64],
    // bishop
    [-14, -21, -11,  -8, -7,  -9, -17, -24,
      -8,  -4,   7, -12, -3, -13,  -4, -14,
       2,  -8,   0,  -1, -2,   6,   0,   4,
      -3,   9,  12,   9, 14,  10,   3,   2,
      -6,   3,  13,  19,  7,  10,  -3,  -9,
     -12,  -3,   8,  10, 13,   3,  -7, -15,
     -14, -18,  -7,  -1,  4,  -9, -15, -27,
     -23,  -9, -23,  -5, -9, -16,  -5, -17],
    // rook
    [13, 10, 18, 15, 12,  12,   8,   5,
     11, 13, 13, 11, -3,   3,   8,   3,
      7,  7,  7,  5,  4,  -3,  -5,  -3,
      4,  3, 13,  1,  2,   1,  -1,   2,
      3,  5,  8,  4, -5,  -6,  -8, -11,
     -4,  0, -5, -1, -7, -12,  -8, -16,
     -6, -6,  0,  2, -9,  -9, -11,  -3,
     -9,  2,  3, -1, -5, -13,   4, -20],
    // queen
    [ -9,  22,  22,  27,  27,  19,  10,  20,
     -17,  20,  32,  41,  58,  25,  30,   0,
     -20,   6,   9,  49,  47,  35,  19,   9,
       3,  22,  24,  45,  57,  40,  57,  36,
     -18,  28,  19,  47,  31,  34,  39,  23,
     -16, -27,  15,   6,   9,  17,  10,   5,
     -22, -23, -30, -16, -16, -23, -36, -32,
     -33, -28, -22, -43,  -5, -32, -20, -41],
    // king
    [-74, -35, -18, -18, -11,  15,   4, -17,
     -12,  17,  14,  17,  17,  38,  23,  11,
      10,  17,  23,  15,  20,  45,  44,  13,
      -8,  22,  24,  27,  26,  33,  26,   3,
     -18,  -4,  21,  24,  27,  23,   9, -11,
     -19,  -3,  11,  21,  23,  16,   7,  -9,
     -27, -11,   4,  13,  14,   4,  -5, -17,
     -53, -34, -21, -11, -28, -14, -24, -43],
];

/// `[color][piece][square]` (square a1 = 0) with piece value folded in. color 0 = white.
const fn build(values: [i32; 6], pst: [[i32; 64]; 6]) -> [[[i32; 64]; 6]; 2] {
    let mut out = [[[0; 64]; 6]; 2];
    let mut p = 0;
    while p < 6 {
        let mut sq = 0;
        while sq < 64 {
            out[0][p][sq] = values[p] + pst[p][sq ^ 56];
            out[1][p][sq] = values[p] + pst[p][sq];
            sq += 1;
        }
        p += 1;
    }
    out
}

static MG_TABLE: [[[i32; 64]; 6]; 2] = build(MG_VALUE, MG_PST);
static EG_TABLE: [[[i32; 64]; 6]; 2] = build(EG_VALUE, EG_PST);

// ---------------------------------------------------------------------------------------------
// Bitboard masks
// ---------------------------------------------------------------------------------------------

const FILE_A: u64 = 0x0101_0101_0101_0101;
const FILE_H: u64 = FILE_A << 7;

const fn file_mask(f: usize) -> u64 {
    FILE_A << f
}

const fn adjacent_files(f: usize) -> u64 {
    let mut m = 0;
    if f > 0 {
        m |= file_mask(f - 1);
    }
    if f < 7 {
        m |= file_mask(f + 1);
    }
    m
}

/// Squares in front of a pawn (own file + adjacent files) — used for passed-pawn detection.
const fn build_passed() -> [[u64; 64]; 2] {
    let mut out = [[0u64; 64]; 2];
    let mut sq = 0;
    while sq < 64 {
        let f = sq % 8;
        let r = sq / 8;
        let files = file_mask(f) | adjacent_files(f);
        // white: ranks above r
        let mut white_ranks = 0u64;
        let mut rr = r + 1;
        while rr < 8 {
            white_ranks |= 0xFFu64 << (rr * 8);
            rr += 1;
        }
        let mut black_ranks = 0u64;
        let mut rr = 0;
        while rr < r {
            black_ranks |= 0xFFu64 << (rr * 8);
            rr += 1;
        }
        out[0][sq] = files & white_ranks;
        out[1][sq] = files & black_ranks;
        sq += 1;
    }
    out
}

static PASSED_MASK: [[u64; 64]; 2] = build_passed();

const PASSED_MG: [i32; 8] = [0, 5, 10, 15, 28, 45, 70, 0];
const PASSED_EG: [i32; 8] = [0, 12, 20, 38, 62, 105, 160, 0];

// mobility weights (per safe square relative to a baseline)
const KNIGHT_MOB: (i32, i32, i32) = (4, 4, 4); // (mg, eg, baseline)
const BISHOP_MOB: (i32, i32, i32) = (5, 5, 6);
const ROOK_MOB: (i32, i32, i32) = (2, 4, 6);
const QUEEN_MOB: (i32, i32, i32) = (1, 2, 12);

#[inline]
fn ci(c: Color) -> usize {
    match c {
        Color::White => 0,
        Color::Black => 1,
    }
}

#[inline]
fn ri(r: Role) -> usize {
    r as usize - 1
}

#[inline]
fn pawn_attacks_bb(color: Color, pawns: u64) -> u64 {
    match color {
        Color::White => ((pawns & !FILE_A) << 7) | ((pawns & !FILE_H) << 9),
        Color::Black => ((pawns & !FILE_A) >> 9) | ((pawns & !FILE_H) >> 7),
    }
}

#[inline]
fn relative_rank(color: Color, sq: Square) -> usize {
    let r = u32::from(sq.rank()) as usize;
    match color {
        Color::White => r,
        Color::Black => 7 - r,
    }
}

#[inline]
fn sq_file(sq: Square) -> usize {
    u32::from(sq.file()) as usize
}

#[inline]
fn sq_rank(sq: Square) -> usize {
    u32::from(sq.rank()) as usize
}

fn center_distance(sq: Square) -> i32 {
    let f = sq_file(sq) as i32;
    let r = sq_rank(sq) as i32;
    let df = if f < 4 { 3 - f } else { f - 4 };
    let dr = if r < 4 { 3 - r } else { r - 4 };
    df + dr
}

fn manhattan(a: Square, b: Square) -> i32 {
    (sq_file(a) as i32 - sq_file(b) as i32).abs() + (sq_rank(a) as i32 - sq_rank(b) as i32).abs()
}

/// Non-pawn material (simple values) for a colour.
fn non_pawn_material(board: &Board, color: Color) -> i32 {
    let us = board.by_color(color);
    (board.knights() & us).count() as i32 * 320
        + (board.bishops() & us).count() as i32 * 330
        + (board.rooks() & us).count() as i32 * 500
        + (board.queens() & us).count() as i32 * 900
}

/// Static evaluation in centipawns from the side-to-move's point of view.
pub fn evaluate(pos: &Chess) -> i32 {
    let board = pos.board();
    let white_pov = evaluate_white(board);
    let tempo = 12;
    let s = match pos.turn() {
        Color::White => white_pov + tempo,
        Color::Black => -white_pov + tempo,
    };
    s.clamp(-EVAL_LIMIT, EVAL_LIMIT)
}

/// Static evaluation from white's point of view (no tempo).
pub fn evaluate_white(board: &Board) -> i32 {
    let mut mg = [0i32; 2];
    let mut eg = [0i32; 2];
    let mut phase = 0;

    let occupied = board.occupied();
    let pawns = [
        (board.pawns() & board.white()).0,
        (board.pawns() & board.black()).0,
    ];
    let pawn_att = [
        pawn_attacks_bb(Color::White, pawns[0]),
        pawn_attacks_bb(Color::Black, pawns[1]),
    ];
    let kings = [board.king_of(Color::White), board.king_of(Color::Black)];

    // King zones (king square + neighbours) for attack counting.
    let zone = [
        kings[0]
            .map(|k| king_attacks(k).0 | (1u64 << u32::from(k)))
            .unwrap_or(0),
        kings[1]
            .map(|k| king_attacks(k).0 | (1u64 << u32::from(k)))
            .unwrap_or(0),
    ];

    let mut attack_units = [0i32; 2]; // attack_units[c] = units colour c directs at the enemy zone
    let mut attackers = [0i32; 2];

    for color in [Color::White, Color::Black] {
        let c = ci(color);
        let o = 1 - c;
        let own = board.by_color(color).0;
        let mob_area = !own & !pawn_att[o];
        let enemy_zone = zone[o];

        for role in Role::ALL {
            let p = ri(role);
            let bb = board.by_piece(Piece { color, role });
            for sq in bb {
                let s = u32::from(sq) as usize;
                mg[c] += MG_TABLE[c][p][s];
                eg[c] += EG_TABLE[c][p][s];
                phase += PHASE_INC[p];

                let (att, units, mob) = match role {
                    Role::Knight => (knight_attacks(sq).0, 2, KNIGHT_MOB),
                    Role::Bishop => (bishop_attacks(sq, occupied).0, 2, BISHOP_MOB),
                    Role::Rook => (rook_attacks(sq, occupied).0, 3, ROOK_MOB),
                    Role::Queen => (queen_attacks(sq, occupied).0, 5, QUEEN_MOB),
                    _ => continue,
                };
                let n = (att & mob_area).count_ones() as i32 - mob.2;
                mg[c] += n * mob.0;
                eg[c] += n * mob.1;
                let za = (att & enemy_zone).count_ones() as i32;
                if za > 0 {
                    attackers[c] += 1;
                    attack_units[c] += units * za;
                }

                if role == Role::Rook {
                    let fm = file_mask(sq_file(sq));
                    if pawns[c] & fm == 0 {
                        if pawns[o] & fm == 0 {
                            mg[c] += 28;
                            eg[c] += 10;
                        } else {
                            mg[c] += 13;
                            eg[c] += 6;
                        }
                    }
                    if relative_rank(color, sq) == 6 {
                        mg[c] += 12;
                        eg[c] += 24;
                    }
                }
            }
        }

        // Bishop pair
        if (board.bishops() & board.by_color(color)).count() >= 2 {
            mg[c] += 30;
            eg[c] += 55;
        }

        // Pawn structure
        let own_pawns = pawns[c];
        let their_pawns = pawns[o];
        for sq in Bitboard(own_pawns) {
            let s = u32::from(sq) as usize;
            let f = sq_file(sq);
            if PASSED_MASK[c][s] & their_pawns == 0 {
                let rr = relative_rank(color, sq);
                // Ignore a pawn that is behind a friendly pawn on the same file (doubled passer).
                let front_same_file = PASSED_MASK[c][s] & file_mask(f) & own_pawns;
                if front_same_file == 0 {
                    mg[c] += PASSED_MG[rr];
                    eg[c] += PASSED_EG[rr];
                    // King proximity in the endgame: own king close, enemy king far from the
                    // square in front of the pawn.
                    if let (Some(ok), Some(ek)) = (kings[c], kings[o]) {
                        let front = match color {
                            Color::White => sq.offset(8),
                            Color::Black => sq.offset(-8),
                        };
                        if let Some(fs) = front {
                            let w = (rr as i32 - 2).max(0);
                            eg[c] += w * (2 * ek.distance(fs) as i32 - ok.distance(fs) as i32) * 2;
                        }
                    }
                }
            }
            if adjacent_files(f) & own_pawns == 0 {
                mg[c] -= 10;
                eg[c] -= 14;
            }
        }
        for f in 0..8 {
            let n = (own_pawns & file_mask(f)).count_ones() as i32;
            if n > 1 {
                mg[c] -= 10 * (n - 1);
                eg[c] -= 22 * (n - 1);
            }
        }

        // King shelter (middlegame only)
        if let Some(k) = kings[c] {
            let kf = sq_file(k);
            let kr = relative_rank(color, k);
            if kr <= 1 {
                let mut shield = 0;
                let lo = kf.saturating_sub(1);
                let hi = (kf + 1).min(7);
                for f in lo..=hi {
                    let fm = file_mask(f);
                    let file_pawns = own_pawns & fm;
                    if file_pawns == 0 {
                        shield -= 18;
                        if their_pawns & fm == 0 {
                            shield -= 10; // fully open file next to the king
                        }
                        continue;
                    }
                    for psq in Bitboard(file_pawns) {
                        let pr = relative_rank(color, psq);
                        if pr == kr + 1 {
                            shield += 14;
                        } else if pr == kr + 2 {
                            shield += 7;
                        }
                    }
                }
                mg[c] += shield;
            } else {
                // King wandering away from home in the middlegame.
                mg[c] -= 8 * kr as i32;
            }
        }
    }

    // King attack penalty (needs at least two attackers, scaled harder with a queen).
    for c in 0..2 {
        let o = 1 - c;
        if attackers[c] >= 2 {
            let color = if c == 0 { Color::White } else { Color::Black };
            let has_queen = (board.queens() & board.by_color(color)).any();
            let u = attack_units[c].min(40);
            let mut pen = u * u / 3;
            if !has_queen {
                pen /= 2;
            }
            mg[o] -= pen.min(450);
        }
    }

    let phase = phase.min(24);
    let mg_score = mg[0] - mg[1];
    let eg_score = eg[0] - eg[1];
    let mut score = (mg_score * phase + eg_score * (24 - phase)) / 24;

    score += mop_up(board);
    scale_drawish(board, score)
}

/// Endgame mop-up: with a decisive material edge against a bare king, drive the enemy king to
/// the edge (or the right corner for KBNK) and bring our king closer. White POV.
fn mop_up(board: &Board) -> i32 {
    for (winner, sign) in [(Color::White, 1), (Color::Black, -1)] {
        let loser = !winner;
        let loser_bb = board.by_color(loser);
        // loser must have a bare king
        if loser_bb.count() != 1 {
            continue;
        }
        let win_bb = board.by_color(winner);
        let npm = non_pawn_material(board, winner);
        let has_pawns = (board.pawns() & win_bb).any();
        if npm < 500 && !(has_pawns && npm > 0) {
            continue;
        }
        let (Some(wk), Some(lk)) = (board.king_of(winner), board.king_of(loser)) else {
            continue;
        };
        let knights = (board.knights() & win_bb).count();
        let bishops = board.bishops() & win_bb;
        let kdist = manhattan(wk, lk);
        let bonus = if npm == 650 && knights == 1 && bishops.count() == 1 && !has_pawns {
            // KBNK: drive to a corner of the bishop's colour.
            let dark = bishops.first().map(|b| b.is_dark()).unwrap_or(true);
            let corners = if dark {
                [Square::A1, Square::H8]
            } else {
                [Square::H1, Square::A8]
            };
            let cd = corners
                .iter()
                .map(|c| lk.distance(*c) as i32)
                .min()
                .unwrap_or(7);
            200 + 60 * (7 - cd) + 12 * (14 - kdist)
        } else {
            300 + 40 * center_distance(lk) + 16 * (14 - kdist)
        };
        return sign * bonus;
    }
    0
}

/// Scale down scores that are hard or impossible to convert (no pawns and little extra material).
fn scale_drawish(board: &Board, score: i32) -> i32 {
    if score == 0 {
        return 0;
    }
    let winner = if score > 0 {
        Color::White
    } else {
        Color::Black
    };
    let win_bb = board.by_color(winner);
    if (board.pawns() & win_bb).any() {
        return score;
    }
    let npm_w = non_pawn_material(board, winner);
    let npm_l = non_pawn_material(board, !winner);
    let win_knights = (board.knights() & win_bb).count();
    if npm_w <= 330 {
        // a lone minor cannot win
        return score / 16;
    }
    if npm_w == 640 && win_knights == 2 && npm_l == 0 {
        return score / 16; // KNNK
    }
    if npm_w - npm_l < 400 {
        return score / 4;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_fen;

    #[test]
    fn startpos_is_balanced() {
        let p = Chess::default();
        let e = evaluate_white(p.board());
        assert!(e.abs() < 30, "startpos eval {e}");
    }

    #[test]
    fn symmetric() {
        // Mirrored positions must evaluate to the negation of each other.
        let fens = [
            "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            "8/5pk1/6p1/8/3P4/8/5PPP/6K1 w - - 0 1",
            "r3k2r/ppq2ppp/2n1b3/3p4/3P4/2N1B3/PPQ2PPP/R3K2R b KQkq - 0 1",
        ];
        for f in fens {
            let p = parse_fen(f).unwrap();
            let mirrored = p.board().clone().into_mirrored();
            assert_eq!(evaluate_white(p.board()), -evaluate_white(&mirrored), "{f}");
        }
    }

    #[test]
    fn material_matters() {
        let up_queen =
            parse_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        assert!(evaluate(&up_queen) > 700);
    }

    #[test]
    fn mop_up_prefers_edge() {
        let center = parse_fen("8/8/8/3k4/8/8/8/4K1Q1 w - - 0 1").unwrap();
        let edge = parse_fen("3k4/8/8/8/8/8/8/4K1Q1 w - - 0 1").unwrap();
        let (e, c) = (evaluate_white(edge.board()), evaluate_white(center.board()));
        assert!(e > c, "edge {e} centre {c}");
    }
}
