//! Opening book built on the fly from `gm_content` openings, matched by Zobrist hash so
//! transpositions are found. Weighted by popularity and the bot's style.

use shakmaty::zobrist::{Zobrist64, ZobristHash};
use shakmaty::{Chess, EnPassantMode, Position};

use gm_content::{Content, Opening};
use gm_engine::uci_to_move;

use crate::personas::Style;

/// Bound on the work done per lookup (openings x plies).
const MAX_SCAN_PLIES: usize = 60;

#[derive(Clone, Debug)]
pub struct BookCandidate {
    pub uci: String,
    pub weight: f64,
    /// Name of the opening this move completes, if any.
    pub name: Option<String>,
}

fn contains_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

/// Style preference multiplier for a whole opening line.
fn style_multiplier(style: Style, o: &Opening) -> f64 {
    let name = o.name.to_ascii_lowercase();
    let fam = o.family.to_ascii_lowercase();
    let text = format!("{name} {fam}");
    match style {
        Style::Aggressive => {
            if contains_any(&text, &["gambit", "attack", "sicilian", "king's indian", "dragon", "fried liver", "evans", "danish", "scotch", "vienna"]) {
                2.5
            } else {
                1.0
            }
        }
        Style::Trappy => {
            let mut m = 1.0;
            if !o.traps.is_empty() {
                m *= 2.5;
            }
            if contains_any(&text, &["gambit", "trap", "fried liver", "stafford", "englund", "budapest"]) {
                m *= 1.8;
            }
            m
        }
        Style::Positional => {
            if contains_any(&text, &["queen's gambit", "london", "english", "catalan", "ruy lopez", "réti", "reti", "caro", "nimzo", "slav", "queen's indian"]) {
                2.2
            } else {
                1.0
            }
        }
        Style::Defensive => {
            if contains_any(&text, &["caro", "french", "slav", "petrov", "petroff", "berlin", "declined", "philidor", "london"]) {
                2.2
            } else {
                1.0
            }
        }
        Style::Beginner | Style::Coach => {
            if o.level.eq_ignore_ascii_case("beginner") {
                2.0
            } else {
                1.0
            }
        }
        Style::Universal => 1.0,
    }
}

fn hash(pos: &Chess) -> u64 {
    pos.zobrist_hash::<Zobrist64>(EnPassantMode::Legal).0
}

/// Book continuations for `pos`, merged by move, highest weight first.
pub fn candidates(content: &Content, pos: &Chess, style: Style) -> Vec<BookCandidate> {
    let target = hash(pos);
    let target_ply_hint = pos.fullmoves().get().saturating_sub(1) as usize * 2;
    if target_ply_hint > MAX_SCAN_PLIES {
        return Vec::new();
    }
    let mut out: Vec<BookCandidate> = Vec::new();
    for o in &content.openings {
        let mut p = Chess::default();
        for (i, u) in o.uci.iter().enumerate().take(MAX_SCAN_PLIES) {
            let Ok(m) = uci_to_move(&p, u) else { break };
            if hash(&p) == target {
                let pop = f64::from(o.popularity.clamp(1, 10));
                let w = pop * pop * style_multiplier(style, o);
                let completes = i + 1 == o.uci.len();
                match out.iter_mut().find(|c| &c.uci == u) {
                    Some(c) => {
                        c.weight += w;
                        if completes && c.name.is_none() {
                            c.name = Some(o.name.clone());
                        }
                    }
                    None => out.push(BookCandidate {
                        uci: u.clone(),
                        weight: w,
                        name: completes.then(|| o.name.clone()),
                    }),
                }
            }
            p.play_unchecked(&m);
        }
    }
    out.sort_by(|a, b| b.weight.total_cmp(&a.weight).then_with(|| a.uci.cmp(&b.uci)));
    out
}

/// Name of the deepest opening whose final position is `pos` (for chat flavor).
pub fn opening_name_at(content: &Content, pos: &Chess) -> Option<String> {
    let target = hash(pos);
    content
        .openings
        .iter()
        .filter(|o| o.uci.len() <= MAX_SCAN_PLIES)
        .filter(|o| {
            let mut p = Chess::default();
            for u in &o.uci {
                let Ok(m) = uci_to_move(&p, u) else { return false };
                p.play_unchecked(&m);
            }
            hash(&p) == target
        })
        .max_by_key(|o| (o.uci.len(), o.popularity))
        .map(|o| o.name.clone())
}
