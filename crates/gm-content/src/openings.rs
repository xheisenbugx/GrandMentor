//! Opening book index: maps every position that occurs along any opening line (keyed by the
//! first four FEN fields, so move clocks don't matter and transpositions merge) to
//!
//! * the opening name for that position — the named opening whose final position it is, or
//!   otherwise the deepest named opening on a line leading to it, and
//! * the book continuations from it, aggregated across all openings and weighted by popularity.
//!
//! The index is built once (O(total plies)) and is immutable afterwards; lookups are a FEN
//! normalization plus one hash probe. Memory is bounded by the size of `openings.json`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use shakmaty::{Chess, Position};

use gm_engine::{fen_key, move_to_san, parse_fen, to_fen, uci_to_move};

use crate::{BookMove, Opening, OpeningMatch, OpeningRef};

/// Name reported for the initial position (which is "in book" but not a named opening).
pub const START_NAME: &str = "Starting Position";
pub const START_ID: &str = "starting-position";

/// Lazily-built, cheaply clonable handle to the opening book. Lives inside `Content`.
#[derive(Clone, Default)]
pub struct OpeningIndex {
    cell: OnceLock<Arc<OpeningBook>>,
}

impl std::fmt::Debug for OpeningIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.cell.get() {
            Some(b) => write!(f, "OpeningIndex({} positions)", b.nodes.len()),
            None => f.write_str("OpeningIndex(unbuilt)"),
        }
    }
}

impl OpeningIndex {
    /// Builds an index for `openings` right away.
    pub fn build(openings: &[Opening]) -> Self {
        let cell = OnceLock::new();
        let _ = cell.set(Arc::new(OpeningBook::build(openings)));
        OpeningIndex { cell }
    }

    /// Returns the book, building it from `openings` on first use.
    pub fn get(&self, openings: &[Opening]) -> &OpeningBook {
        self.cell.get_or_init(|| Arc::new(OpeningBook::build(openings)))
    }
}

#[derive(Debug, Clone)]
struct Cont {
    uci: String,
    san: String,
    weight: u32,
    child: u32,
}

#[derive(Debug, Clone, Default)]
struct Node {
    /// Opening whose final position this is (best one if several transpose here).
    exact: Option<u32>,
    /// Deepest named opening along any book line reaching this position.
    inherited: Option<u32>,
    /// Sorted by weight (desc), then UCI.
    conts: Vec<Cont>,
}

/// Immutable opening book (see module docs).
#[derive(Debug, Clone)]
pub struct OpeningBook {
    keys: HashMap<String, u32>,
    nodes: Vec<Node>,
    /// Name data copied from the openings so the book never indexes into a mutable Vec.
    refs: Vec<OpeningRef>,
    /// (plies, popularity) per entry in `refs`, used to rank candidates.
    rank: Vec<(usize, u8)>,
    start: u32,
}

impl OpeningBook {
    pub fn build(openings: &[Opening]) -> Self {
        let mut book = OpeningBook {
            keys: HashMap::with_capacity(openings.len() * 8),
            nodes: Vec::with_capacity(openings.len() * 8),
            refs: Vec::with_capacity(openings.len()),
            rank: Vec::with_capacity(openings.len()),
            start: 0,
        };
        let start_pos = Chess::default();
        book.start = book.node_for(fen_key(&to_fen(&start_pos)));

        // Pass 1: walk every line, creating nodes and aggregating continuations.
        let mut paths: Vec<(u32, Vec<u32>)> = Vec::with_capacity(openings.len());
        for o in openings {
            let oi = u32::try_from(book.refs.len()).unwrap_or(u32::MAX);
            book.refs.push(OpeningRef { id: o.id.clone(), eco: o.eco.clone(), name: o.name.clone() });
            book.rank.push((o.uci.len(), o.popularity));
            let weight = u32::from(o.popularity.clamp(1, 10));

            let mut pos = start_pos.clone();
            let mut cur = book.start;
            let mut path = Vec::with_capacity(o.uci.len());
            let mut complete = true;
            for u in &o.uci {
                let Ok(m) = uci_to_move(&pos, u) else {
                    complete = false;
                    break;
                };
                let san = move_to_san(&pos, &m);
                pos.play_unchecked(&m);
                let child = book.node_for(fen_key(&to_fen(&pos)));
                let conts = &mut book.nodes[cur as usize].conts;
                match conts.iter_mut().find(|c| c.uci == *u) {
                    Some(c) => c.weight = c.weight.saturating_add(weight),
                    None => conts.push(Cont { uci: u.clone(), san, weight, child }),
                }
                path.push(child);
                cur = child;
            }
            if complete && !path.is_empty() {
                let better = match book.nodes[cur as usize].exact {
                    None => true,
                    Some(prev) => book.better_exact(oi, prev),
                };
                if better {
                    book.nodes[cur as usize].exact = Some(oi);
                }
                paths.push((oi, path));
            }
        }

        // Pass 2: propagate names down each line so mid-line positions get the deepest named
        // opening on a path to them (transpositions take the most specific candidate).
        for (_, path) in &paths {
            let mut current: Option<u32> = None;
            for &n in path {
                if let Some(e) = book.nodes[n as usize].exact {
                    current = Some(e);
                }
                if let Some(c) = current {
                    let better = match book.nodes[n as usize].inherited {
                        None => true,
                        Some(prev) => book.better_inherited(c, prev),
                    };
                    if better {
                        book.nodes[n as usize].inherited = Some(c);
                    }
                }
            }
        }

        for n in &mut book.nodes {
            n.conts.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.uci.cmp(&b.uci)));
            n.conts.shrink_to_fit();
        }
        book.keys.shrink_to_fit();
        book.nodes.shrink_to_fit();
        book
    }

    fn node_for(&mut self, key: String) -> u32 {
        if let Some(&i) = self.keys.get(&key) {
            return i;
        }
        let i = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push(Node::default());
        self.keys.insert(key, i);
        i
    }

    /// Two openings ending on the same position: prefer the more popular, then the shorter
    /// (canonical) move order, then the id for determinism.
    fn better_exact(&self, a: u32, b: u32) -> bool {
        let (ra, rb) = (self.rank[a as usize], self.rank[b as usize]);
        (rb.1, ra.0, &self.refs[a as usize].id) < (ra.1, rb.0, &self.refs[b as usize].id)
    }

    /// Inherited names: prefer the deepest (most specific) line, then popularity, then id.
    fn better_inherited(&self, a: u32, b: u32) -> bool {
        let (ra, rb) = (self.rank[a as usize], self.rank[b as usize]);
        (rb.0, rb.1, &self.refs[a as usize].id) < (ra.0, ra.1, &self.refs[b as usize].id)
    }

    fn node_of_fen(&self, fen: &str) -> Option<&Node> {
        let pos = parse_fen(fen).ok()?;
        self.node_of_pos(&pos)
    }

    fn node_of_pos(&self, pos: &Chess) -> Option<&Node> {
        let i = *self.keys.get(&fen_key(&to_fen(pos)))?;
        self.nodes.get(i as usize)
    }

    /// Number of distinct book positions.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// See `Content::lookup_opening`.
    pub fn lookup(&self, fen: &str) -> Option<OpeningMatch> {
        let pos = parse_fen(fen).ok()?;
        self.lookup_position(&pos)
    }

    /// Same as [`lookup`](Self::lookup) for an already parsed position.
    pub fn lookup_position(&self, pos: &Chess) -> Option<OpeningMatch> {
        let i = *self.keys.get(&fen_key(&to_fen(pos)))?;
        let node = self.nodes.get(i as usize)?;
        let opening = match node.exact.or(node.inherited) {
            Some(o) => self.refs.get(o as usize)?.clone(),
            None if i == self.start => {
                OpeningRef { id: START_ID.to_string(), eco: String::new(), name: START_NAME.to_string() }
            }
            None => return None,
        };
        let continuations = node
            .conts
            .iter()
            .map(|c| BookMove {
                uci: c.uci.clone(),
                san: c.san.clone(),
                name: self
                    .nodes
                    .get(c.child as usize)
                    .and_then(|n| n.exact)
                    .and_then(|o| self.refs.get(o as usize))
                    .map(|r| r.name.clone()),
                weight: c.weight,
            })
            .collect();
        Some(OpeningMatch { opening, continuations })
    }

    /// True if the position occurs along any book line (including the start position).
    pub fn contains(&self, fen: &str) -> bool {
        self.node_of_fen(fen).is_some()
    }

    /// Same as [`contains`](Self::contains) for an already parsed position.
    pub fn contains_position(&self, pos: &Chess) -> bool {
        self.node_of_pos(pos).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Content;
    use std::path::PathBuf;

    fn content() -> Content {
        Content::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")).expect("load")
    }

    fn fen_after(moves: &str) -> String {
        let (pos, _) = crate::replay_san(&Chess::default(), moves).expect("legal");
        to_fen(&pos)
    }

    #[test]
    fn dataset_is_rich_and_well_formed() {
        let c = content();
        assert!(c.openings.len() >= 150, "only {} openings", c.openings.len());
        let mut families = std::collections::HashSet::new();
        for o in &c.openings {
            assert!(o.eco.len() == 3 && matches!(o.eco.as_bytes()[0], b'A'..=b'E'), "{}: eco {}", o.id, o.eco);
            assert!(o.side == "white" || o.side == "black", "{}: side", o.id);
            assert!(matches!(o.level.as_str(), "beginner" | "intermediate" | "advanced"), "{}: level", o.id);
            assert!((1..=10).contains(&o.popularity), "{}: popularity", o.id);
            assert!(!o.family.is_empty() && !o.description.is_empty(), "{}: text", o.id);
            assert!(o.ideas.len() >= 3, "{}: ideas", o.id);
            families.insert(o.eco.as_bytes()[0]);
        }
        assert_eq!(families.len(), 5, "all ECO volumes A-E covered");
    }

    #[test]
    fn exact_names_and_continuations() {
        let c = content();
        let m = c.lookup_opening(&fen_after("e4 e5 Nf3 Nc6 Bc4")).expect("italian");
        assert_eq!(m.opening.id, "italian-game");
        let sans: Vec<&str> = m.continuations.iter().map(|b| b.san.as_str()).collect();
        assert!(sans.contains(&"Bc5") && sans.contains(&"Nf6"), "{sans:?}");
        let bc5 = m.continuations.iter().find(|b| b.san == "Bc5").expect("Bc5");
        assert_eq!(bc5.name.as_deref(), Some("Giuoco Piano"));
        assert_eq!(bc5.uci, "f8c5");
        // Sorted by weight.
        assert!(m.continuations.windows(2).all(|w| w[0].weight >= w[1].weight));
    }

    #[test]
    fn start_position_has_book_moves() {
        let c = content();
        let m = c.lookup_opening(gm_engine::START_FEN).expect("start");
        assert_eq!(m.opening.id, START_ID);
        assert_eq!(m.continuations.len(), 20, "every first move is named");
        assert!(matches!(m.continuations[0].san.as_str(), "e4" | "d4"));
        assert!(c.is_book_position(gm_engine::START_FEN));
    }

    #[test]
    fn mid_line_positions_inherit_deepest_name() {
        let c = content();
        // Inside the Najdorf English Attack line, before its final move.
        let m = c.lookup_opening(&fen_after("e4 c5 Nf3 d6 d4 cxd4 Nxd4 Nf6 Nc3 a6 Be3 e5 Nb3")).expect("book");
        assert_eq!(m.opening.id, "sicilian-najdorf");
        assert!(c.is_book_position(&fen_after("e4 c5 Nf3 d6 d4 cxd4 Nxd4 Nf6 Nc3 a6 Be3 e5 Nb3")));
    }

    #[test]
    fn transpositions_merge() {
        let c = content();
        // Caro-Kann Classical via 3.Nd2 instead of 3.Nc3.
        let m = c.lookup_opening(&fen_after("e4 c6 d4 d5 Nd2 dxe4 Nxe4 Bf5")).expect("transposed");
        assert_eq!(m.opening.id, "caro-kann-classical");
        // Move-order independence: Nf3 d5 d4 = d4 d5 Nf3.
        let a = c.lookup_opening(&fen_after("Nf3 d5 d4")).expect("a");
        let b = c.lookup_opening(&fen_after("d4 d5 Nf3")).expect("b");
        assert_eq!(a.opening.id, b.opening.id);
    }

    #[test]
    fn clocks_ignored_and_out_of_book() {
        let c = content();
        let f = fen_after("e4 e5 Nf3 Nc6 Bb5");
        let mut parts: Vec<&str> = f.split(' ').collect();
        parts[4] = "7";
        parts[5] = "42";
        let altered = parts.join(" ");
        assert_eq!(c.lookup_opening(&altered).map(|m| m.opening.id).as_deref(), Some("ruy-lopez"));
        assert!(c.lookup_opening(&fen_after("e4 e5 Ke2 Ke7")).is_none());
        assert!(!c.is_book_position(&fen_after("e4 e5 Ke2 Ke7")));
        assert!(c.lookup_opening("garbage").is_none());
        assert!(c.is_book_position("startpos")); // gm_engine::parse_fen accepts start aliases
        assert!(!c.is_book_position("8/8/8/8/8/8/8/8 w - - 0 1"));
    }

    #[test]
    fn every_opening_resolves_to_itself_or_a_transposition() {
        let c = content();
        for o in &c.openings {
            let m = c.lookup_opening(&o.fen).unwrap_or_else(|| panic!("{} not found", o.id));
            let named = c.opening(&m.opening.id).expect("named opening exists");
            assert_eq!(gm_engine::fen_key(&named.fen), gm_engine::fen_key(&o.fen), "{}", o.id);
        }
    }

    #[test]
    fn default_content_builds_lazily() {
        let mut c = Content::default();
        assert!(c.lookup_opening(gm_engine::START_FEN).is_some()); // empty book still knows the start
        c.openings = content().openings;
        c.rebuild_opening_index();
        assert!(c.lookup_opening(&fen_after("d4 d5 c4")).is_some());
    }
}
