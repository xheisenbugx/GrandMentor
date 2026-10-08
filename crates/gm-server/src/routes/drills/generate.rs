//! Question generators for the quick drills (pure functions of the puzzle set and an RNG).
//!
//! Positions come from the puzzle collection (the starting position and every position along the
//! solution line); when that is not enough (tiny content sets) random legal positions fill in.
//! Every generator is bounded by a fixed number of attempts.

use std::collections::{HashSet, VecDeque};

use gm_content::Puzzle;
use gm_engine::{move_to_san, move_to_uci, parse_fen, to_fen};
use rand::seq::SliceRandom;
use rand::Rng;
use serde::Serialize;
use shakmaty::{attacks, Bitboard, Board, Chess, Color, Move, Position, Role, Square};

/// Largest batch a client may ask for.
pub const MAX_BATCH: usize = 50;
/// Puzzle-position attempts per requested item.
const PUZZLE_TRIES_PER_ITEM: usize = 80;
/// Random-position attempts per requested item (fallback).
const RANDOM_TRIES_PER_ITEM: usize = 200;

// ---------------------------------------------------------------------------------------------
// Item shapes (wire format, see docs/CONTRACT.md "Quick drills")
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HangingPiece {
    pub square: String,
    /// Piece code like `wN` / `bQ`.
    pub piece: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HangingItem {
    pub fen: String,
    pub hanging: Vec<HangingPiece>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MaterialItem {
    pub fen: String,
    /// Material in pawns (P=1, N=3, B=3, R=5, Q=9).
    pub white: i32,
    pub black: i32,
    /// white - black
    pub diff: i32,
    /// Four distinct candidate answers (diffs), shuffled; one is `diff`.
    pub options: Vec<i32>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MoveAnswer {
    pub uci: String,
    pub san: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChecksItem {
    pub fen: String,
    /// Every legal move that gives check (or captures, for the `captures` variant).
    pub answers: Vec<MoveAnswer>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct KnightItem {
    /// Board to show: the white knight plus (advanced) black pieces. No kings.
    pub fen: String,
    pub start: String,
    pub target: String,
    pub min_moves: u32,
    /// Squares the knight may not land on (occupied or attacked by the black pieces).
    pub blocked: Vec<String>,
    /// One shortest route, start excluded, target included.
    pub path: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Material & exchange helpers
// ---------------------------------------------------------------------------------------------

/// Teaching values in pawns.
pub fn points(role: Role) -> i32 {
    match role {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 0,
    }
}

fn see_value(role: Role) -> i32 {
    match role {
        Role::King => 20_000,
        r => points(r) * 100,
    }
}

const SWAP_ORDER: [Role; 6] = [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen, Role::King];

/// Static exchange evaluation of a capture from the capturer's point of view (centipawns).
pub fn see(board: &Board, m: &Move) -> i32 {
    let (Some(from), Some(captured)) = (m.from(), m.capture()) else { return 0 };
    let to = m.to();
    let Some(mover) = board.color_at(from) else { return 0 };
    let mut occupied = board.occupied();
    occupied.discard(from);
    if m.is_en_passant() {
        occupied.discard(Square::from_coords(to.file(), from.rank()));
    }
    let mut gains = [0i32; 34];
    gains[0] = see_value(captured);
    let mut last = see_value(m.promotion().unwrap_or(m.role()));
    let mut side = !mover;
    let mut depth = 0usize;
    while depth + 1 < gains.len() {
        let attackers = board.attacks_to(to, side, occupied) & occupied;
        let Some((sq, role)) = SWAP_ORDER
            .iter()
            .find_map(|&r| (attackers & board.by_role(r)).first().map(|sq| (sq, r)))
        else {
            break;
        };
        depth += 1;
        gains[depth] = last - gains[depth - 1];
        if gains[depth].max(-gains[depth - 1]) < 0 {
            break;
        }
        last = see_value(role);
        occupied.discard(sq);
        side = !side;
    }
    while depth > 0 {
        gains[depth - 1] = -((-gains[depth - 1]).max(gains[depth]));
        depth -= 1;
    }
    gains[0]
}

/// Material of `color` in pawns (kings excluded).
pub fn material(board: &Board, color: Color) -> i32 {
    [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen]
        .iter()
        .map(|&r| (board.by_role(r) & board.by_color(color)).count() as i32 * points(r))
        .sum()
}

fn piece_code(color: Color, role: Role) -> String {
    format!("{}{}", if color == Color::White { 'w' } else { 'b' }, role.upper_char())
}

/// Pieces (both colours, never kings) that the other side can capture and come out ahead:
/// a legal capture onto the piece's square with a positive static exchange. Returns None when
/// the position is unsuitable (a side is in check, so "the other side to move" is illegal).
pub fn hanging_pieces(pos: &Chess) -> Option<Vec<HangingPiece>> {
    if pos.is_check() {
        return None;
    }
    let other = pos.clone().swap_turn().ok()?;
    let mut found: Vec<(Square, HangingPiece)> = Vec::new();
    for p in [pos, &other] {
        let board = p.board();
        for m in p.legal_moves().iter() {
            if !m.is_capture() || m.is_en_passant() || m.capture() == Some(Role::King) {
                continue;
            }
            let to = m.to();
            if found.iter().any(|(sq, _)| *sq == to) || see(board, m) <= 0 {
                continue;
            }
            if let Some(piece) = board.piece_at(to) {
                found.push((to, HangingPiece { square: to.to_string(), piece: piece_code(piece.color, piece.role) }));
            }
        }
    }
    found.sort_by_key(|(sq, _)| u32::from(*sq));
    Some(found.into_iter().map(|(_, h)| h).collect())
}

/// Legal moves that give check (`captures == false`) or that capture (`captures == true`).
pub fn target_moves(pos: &Chess, captures: bool) -> Vec<MoveAnswer> {
    let mut out: Vec<MoveAnswer> = pos
        .legal_moves()
        .iter()
        .filter(|m| {
            if captures {
                m.is_capture()
            } else {
                pos.clone().play(m).map(|p| p.is_check()).unwrap_or(false)
            }
        })
        .map(|m| MoveAnswer { uci: move_to_uci(m), san: move_to_san(pos, m) })
        .collect();
    out.sort_by(|a, b| a.uci.cmp(&b.uci));
    out
}

// ---------------------------------------------------------------------------------------------
// Position sources
// ---------------------------------------------------------------------------------------------

/// A random position from the puzzle set: the puzzle start or any position along its line.
fn puzzle_position<R: Rng>(puzzles: &[Puzzle], rng: &mut R) -> Option<Chess> {
    let p = puzzles.choose(rng)?;
    let mut pos = parse_fen(&p.fen).ok()?;
    let plies = rng.gen_range(0..=p.moves.len().min(12));
    for uci in p.moves.iter().take(plies) {
        let m = gm_engine::uci_to_move(&pos, uci).ok()?;
        pos = pos.play(&m).ok()?;
    }
    Some(pos)
}

/// A random legal position with both kings and a handful of pieces (fallback source).
fn random_position<R: Rng>(rng: &mut R) -> Option<Chess> {
    const POOL: [char; 12] = ['Q', 'R', 'R', 'B', 'B', 'N', 'N', 'P', 'P', 'P', 'P', 'P'];
    let mut grid: [Option<char>; 64] = [None; 64];
    let mut free: Vec<usize> = (0..64).collect();
    free.shuffle(rng);
    let mut pieces = vec!['K', 'k'];
    for _ in 0..rng.gen_range(5..=12) {
        let c = *POOL.choose(rng)?;
        pieces.push(if rng.gen_bool(0.5) { c.to_ascii_lowercase() } else { c });
    }
    for c in pieces {
        let pawn = c.eq_ignore_ascii_case(&'p');
        if let Some(k) = free.iter().position(|&i| !pawn || (8..56).contains(&i)) {
            grid[free.swap_remove(k)] = Some(c);
        }
    }
    let turn = if rng.gen_bool(0.5) { 'w' } else { 'b' };
    parse_fen(&format!("{} {turn} - - 0 1", placement(&grid))).ok()
}

/// FEN placement field from a 64-entry grid indexed a1=0 .. h8=63.
fn placement(grid: &[Option<char>; 64]) -> String {
    let mut s = String::with_capacity(72);
    for rank in (0..8).rev() {
        let mut empty = 0;
        for file in 0..8 {
            match grid[rank * 8 + file] {
                Some(c) => {
                    if empty > 0 {
                        s.push(char::from(b'0' + empty));
                        empty = 0;
                    }
                    s.push(c);
                }
                None => empty += 1,
            }
        }
        if empty > 0 {
            s.push(char::from(b'0' + empty));
        }
        if rank > 0 {
            s.push('/');
        }
    }
    s
}

/// Generic "draw positions until `n` items pass `accept`" loop with bounded attempts and
/// de-duplication by position.
fn collect<T, R, F>(puzzles: &[Puzzle], n: usize, rng: &mut R, mut accept: F) -> Vec<T>
where
    R: Rng,
    F: FnMut(&Chess, &mut R) -> Option<T>,
{
    let n = n.clamp(1, MAX_BATCH);
    let mut out = Vec::with_capacity(n);
    let mut seen = HashSet::new();
    let mut try_pos = |pos: Chess, rng: &mut R, out: &mut Vec<T>| {
        let key = gm_engine::fen_key(&to_fen(&pos));
        if seen.contains(&key) {
            return;
        }
        if let Some(item) = accept(&pos, rng) {
            seen.insert(key);
            out.push(item);
        }
    };
    if !puzzles.is_empty() {
        for _ in 0..n * PUZZLE_TRIES_PER_ITEM {
            if out.len() >= n {
                break;
            }
            if let Some(pos) = puzzle_position(puzzles, rng) {
                try_pos(pos, rng, &mut out);
            }
        }
    }
    for _ in 0..n * RANDOM_TRIES_PER_ITEM {
        if out.len() >= n {
            break;
        }
        if let Some(pos) = random_position(rng) {
            try_pos(pos, rng, &mut out);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------------------------

/// Positions with 1..=3 hanging pieces.
pub fn hanging_batch<R: Rng>(puzzles: &[Puzzle], n: usize, rng: &mut R) -> Vec<HangingItem> {
    collect(puzzles, n, rng, |pos, _| {
        let hanging = hanging_pieces(pos)?;
        (1..=3).contains(&hanging.len()).then(|| HangingItem { fen: to_fen(pos), hanging })
    })
}

/// "Who is ahead and by how much": mostly unbalanced positions, a few equal ones.
pub fn material_batch<R: Rng>(puzzles: &[Puzzle], n: usize, rng: &mut R) -> Vec<MaterialItem> {
    collect(puzzles, n, rng, |pos, rng| {
        let board = pos.board();
        let (white, black) = (material(board, Color::White), material(board, Color::Black));
        let diff = white - black;
        if diff.abs() > 12 || (diff == 0 && !rng.gen_bool(0.2)) {
            return None;
        }
        Some(MaterialItem { fen: to_fen(pos), white, black, diff, options: material_options(diff, rng) })
    })
}

/// The right answer plus three plausible, distinct distractors (shuffled).
pub fn material_options<R: Rng>(diff: i32, rng: &mut R) -> Vec<i32> {
    let mut cands: Vec<i32> = vec![-diff, diff + 1, diff - 1, diff + 2, diff - 2, diff + 3, diff - 3, 0];
    cands.retain(|&c| c != diff && c.abs() <= 15);
    cands.sort_unstable();
    cands.dedup();
    cands.shuffle(rng);
    let mut opts = vec![diff];
    opts.extend(cands.into_iter().take(3));
    opts.shuffle(rng);
    opts
}

/// Positions (side to move not in check, no promotions available) with 1..=6 checking moves
/// (`captures == false`) or 1..=6 captures (`captures == true`).
pub fn checks_batch<R: Rng>(puzzles: &[Puzzle], n: usize, captures: bool, rng: &mut R) -> Vec<ChecksItem> {
    collect(puzzles, n, rng, |pos, _| {
        if pos.is_check() || pos.is_game_over() {
            return None;
        }
        let legal = pos.legal_moves();
        if legal.iter().any(|m| m.is_promotion()) {
            return None;
        }
        let answers = target_moves(pos, captures);
        (1..=6).contains(&answers.len()).then(|| ChecksItem { fen: to_fen(pos), answers })
    })
}

/// Breadth-first knight distance from `start` to `target`, never landing on `blocked`.
/// Returns the shortest path (start excluded) or None if unreachable.
pub fn knight_path(start: Square, target: Square, blocked: Bitboard) -> Option<Vec<Square>> {
    let mut prev: [Option<Square>; 64] = [None; 64];
    let mut visited = Bitboard::from_square(start);
    let mut queue = VecDeque::from([start]);
    while let Some(sq) = queue.pop_front() {
        if sq == target {
            let mut path = vec![target];
            let mut cur = target;
            while let Some(p) = prev[usize::from(cur)] {
                if p == start {
                    break;
                }
                path.push(p);
                cur = p;
            }
            path.reverse();
            return Some(path);
        }
        for next in attacks::knight_attacks(sq) {
            if visited.contains(next) || blocked.contains(next) {
                continue;
            }
            visited.add(next);
            prev[usize::from(next)] = Some(sq);
            queue.push_back(next);
        }
    }
    None
}

/// Knight routes. `advanced` adds 2..=4 black pieces whose squares and attacks are off-limits.
pub fn knight_batch<R: Rng>(n: usize, advanced: bool, rng: &mut R) -> Vec<KnightItem> {
    let n = n.clamp(1, MAX_BATCH);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n * RANDOM_TRIES_PER_ITEM {
        if out.len() >= n {
            break;
        }
        if let Some(item) = knight_item(advanced, rng) {
            if !out.iter().any(|o: &KnightItem| o.start == item.start && o.target == item.target && o.fen == item.fen) {
                out.push(item);
            }
        }
    }
    out
}

fn knight_item<R: Rng>(advanced: bool, rng: &mut R) -> Option<KnightItem> {
    let mut grid: [Option<char>; 64] = [None; 64];
    let mut blocked = Bitboard::EMPTY;
    if advanced {
        let mut occupied = Bitboard::EMPTY;
        let mut pieces: Vec<(Square, Role)> = Vec::new();
        for _ in 0..rng.gen_range(2..=4) {
            let role = *[Role::Pawn, Role::Pawn, Role::Knight, Role::Bishop, Role::Rook].choose(rng)?;
            let idx = if role == Role::Pawn { rng.gen_range(8..56) } else { rng.gen_range(0..64) };
            let sq = Square::new(idx);
            if occupied.contains(sq) {
                continue;
            }
            occupied.add(sq);
            pieces.push((sq, role));
            grid[idx as usize] = Some(role.char());
        }
        blocked = occupied;
        for (sq, role) in pieces {
            blocked |= match role {
                Role::Pawn => attacks::pawn_attacks(Color::Black, sq),
                Role::Knight => attacks::knight_attacks(sq),
                Role::Bishop => attacks::bishop_attacks(sq, occupied),
                _ => attacks::rook_attacks(sq, occupied),
            };
        }
        if blocked.count() > 40 {
            return None;
        }
    }
    let start = Square::new(rng.gen_range(0..64));
    let target = Square::new(rng.gen_range(0..64));
    if start == target || blocked.contains(start) || blocked.contains(target) {
        return None;
    }
    let path = knight_path(start, target, blocked)?;
    let max = if advanced { 6 } else { 5 };
    if !(2..=max).contains(&path.len()) {
        return None;
    }
    grid[usize::from(start)] = Some('N');
    Some(KnightItem {
        fen: format!("{} w - - 0 1", placement(&grid)),
        start: start.to_string(),
        target: target.to_string(),
        min_moves: path.len() as u32,
        blocked: blocked.into_iter().map(|s| s.to_string()).collect(),
        path: path.iter().map(|s| s.to_string()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use std::path::PathBuf;

    fn puzzles() -> Vec<Puzzle> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        gm_content::Content::load(&dir).expect("content").puzzles
    }

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    #[test]
    fn hanging_detects_simple_cases() {
        // Black knight on d5 attacked by the d1 rook and undefended.
        let pos = parse_fen("4k3/8/8/3n4/8/8/8/3RK3 w - - 0 1").unwrap();
        let h = hanging_pieces(&pos).unwrap();
        assert_eq!(h, vec![HangingPiece { square: "d5".into(), piece: "bN".into() }]);
        // Defended by a pawn and attacked only by the rook: not hanging.
        let pos = parse_fen("4k3/8/4p3/3n4/8/8/8/3RK3 w - - 0 1").unwrap();
        assert!(hanging_pieces(&pos).unwrap().is_empty());
        // Defended but attacked by something cheaper: hanging (en prise).
        let pos = parse_fen("4k3/8/4p3/3q4/2P5/8/8/4K3 w - - 0 1").unwrap();
        assert!(hanging_pieces(&pos).unwrap().iter().any(|h| h.square == "d5"));
        // Works for the side to move too: white queen on d4 attacked by the black knight on b5.
        let pos = parse_fen("4k3/8/8/1n6/3Q4/8/8/4K3 w - - 0 1").unwrap();
        let h = hanging_pieces(&pos).unwrap();
        assert!(h.iter().any(|h| h.square == "d4" && h.piece == "wQ"));
        // In check: unsuitable.
        assert!(hanging_pieces(&parse_fen("4k3/8/8/8/8/8/8/R3K2r w - - 0 1").unwrap()).is_none());
    }

    #[test]
    fn hanging_batch_positions_really_have_hanging_pieces() {
        let ps = puzzles();
        let batch = hanging_batch(&ps, 30, &mut rng());
        assert_eq!(batch.len(), 30);
        for item in &batch {
            let pos = parse_fen(&item.fen).unwrap();
            assert!(!item.hanging.is_empty() && item.hanging.len() <= 3);
            assert_eq!(hanging_pieces(&pos).unwrap(), item.hanging);
            for h in &item.hanging {
                // Independently: some enemy legal capture of that square wins material.
                let sq: Square = h.square.parse().unwrap();
                let piece = pos.board().piece_at(sq).unwrap();
                assert_ne!(piece.role, Role::King);
                let attacker = if pos.turn() == !piece.color { pos.clone() } else { pos.clone().swap_turn().unwrap() };
                assert!(attacker
                    .legal_moves()
                    .iter()
                    .any(|m| m.to() == sq && m.is_capture() && see(attacker.board(), m) > 0));
            }
        }
    }

    #[test]
    fn checks_answers_equal_the_legal_checking_moves() {
        let ps = puzzles();
        for captures in [false, true] {
            let batch = checks_batch(&ps, 25, captures, &mut rng());
            assert_eq!(batch.len(), 25);
            for item in &batch {
                let pos = parse_fen(&item.fen).unwrap();
                let mut expected: Vec<String> = pos
                    .legal_moves()
                    .iter()
                    .filter(|m| {
                        if captures {
                            m.capture().is_some()
                        } else {
                            let after = pos.clone().play(m).unwrap();
                            after.is_check()
                        }
                    })
                    .map(move_to_uci)
                    .collect();
                expected.sort();
                let got: Vec<String> = item.answers.iter().map(|a| a.uci.clone()).collect();
                assert_eq!(got, expected, "{}", item.fen);
                if !captures {
                    assert!(item.answers.iter().all(|a| a.san.ends_with('+') || a.san.ends_with('#')));
                }
            }
        }
    }

    #[test]
    fn material_items_are_consistent() {
        let ps = puzzles();
        let mut r = rng();
        let batch = material_batch(&ps, 40, &mut r);
        assert_eq!(batch.len(), 40);
        for item in &batch {
            let pos = parse_fen(&item.fen).unwrap();
            assert_eq!(item.white, material(pos.board(), Color::White));
            assert_eq!(item.black, material(pos.board(), Color::Black));
            assert_eq!(item.diff, item.white - item.black);
            assert_eq!(item.options.len(), 4);
            assert!(item.options.contains(&item.diff));
            let uniq: HashSet<i32> = item.options.iter().copied().collect();
            assert_eq!(uniq.len(), 4);
        }
        assert_eq!(material(Chess::default().board(), Color::White), 39);
    }

    #[test]
    fn knight_routes_are_shortest_and_avoid_blocked_squares() {
        let a1 = Square::A1;
        assert_eq!(knight_path(a1, Square::B3, Bitboard::EMPTY).unwrap(), vec![Square::B3]);
        assert_eq!(knight_path(a1, Square::H8, Bitboard::EMPTY).unwrap().len(), 6);
        assert_eq!(knight_path(a1, Square::B2, Bitboard::EMPTY).unwrap().len(), 4);
        for advanced in [false, true] {
            let batch = knight_batch(30, advanced, &mut rng());
            assert_eq!(batch.len(), 30);
            for item in &batch {
                let blocked: Bitboard = item.blocked.iter().map(|s| s.parse::<Square>().unwrap()).collect();
                let start: Square = item.start.parse().unwrap();
                let target: Square = item.target.parse().unwrap();
                assert!(!blocked.contains(start) && !blocked.contains(target));
                let path = knight_path(start, target, blocked).unwrap();
                assert_eq!(path.len() as u32, item.min_moves);
                assert_eq!(item.path.len() as u32, item.min_moves);
                // The served route is legal hop by hop.
                let mut cur = start;
                for s in &item.path {
                    let sq: Square = s.parse().unwrap();
                    assert!(attacks::knight_attacks(cur).contains(sq) && !blocked.contains(sq));
                    cur = sq;
                }
                assert_eq!(cur, target);
                assert_eq!(advanced, !item.blocked.is_empty());
            }
        }
    }

    #[test]
    fn falls_back_to_random_positions_without_puzzles() {
        let mut r = rng();
        assert_eq!(hanging_batch(&[], 5, &mut r).len(), 5);
        assert_eq!(material_batch(&[], 5, &mut r).len(), 5);
        assert_eq!(checks_batch(&[], 5, false, &mut r).len(), 5);
    }
}
