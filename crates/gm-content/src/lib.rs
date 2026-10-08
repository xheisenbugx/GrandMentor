//! gm-content: loads & validates `data/*.json` (openings, puzzles, courses, endgames) and
//! answers opening-book queries. Types are defined by `docs/CONTRACT.md` §3.
//!
//! Also home of [`Lang`] (the user-facing language shared by every crate), the chess
//! vocabulary helpers in [`words`], and the translation overlays in `data/i18n/<lang>/`
//! (see [`Content::localized`] and `docs/I18N.md`).

pub mod classics;
mod lang;
mod openings;
pub mod overlay;
pub mod steps;
pub mod words;

use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};

use anyhow::Context as _;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Position};

use gm_engine::{move_to_uci, parse_fen, san_to_move, to_fen, uci_to_move};

pub use classics::{Classic, ClassicNote, ClassicQuestion, ClassicSummary};
pub use lang::Lang;
pub use openings::{start_name, OpeningBook, OpeningIndex, START_ID, START_NAME};
pub use overlay::{Overlay, OverlayStats};

// ---------------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Opening {
    pub id: String,
    pub eco: String,
    pub name: String,
    pub family: String,
    /// SAN, space separated, no move numbers: "e4 e5 Nf3 Nc6 Bb5"
    pub moves: String,
    /// "white" | "black" = whose repertoire
    pub side: String,
    /// 1-10
    pub popularity: u8,
    /// beginner | intermediate | advanced
    pub level: String,
    pub description: String,
    pub ideas: Vec<String>,
    pub traps: Vec<String>,
    /// filled by loader
    pub uci: Vec<String>,
    /// filled by loader: final position
    pub fen: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Puzzle {
    pub id: String,
    pub fen: String,
    /// UCI. moves[0] = opponent's move played BEFORE the user is on move (lichess convention);
    /// the user plays moves[1], moves[3], ...
    pub moves: Vec<String>,
    pub rating: u16,
    pub themes: Vec<String>,
    pub popularity: i16,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Course {
    pub id: String,
    pub title: String,
    /// basics | openings | middlegame | tactics | strategy | endgame
    pub category: String,
    pub level: String,
    pub description: String,
    /// emoji
    pub icon: String,
    pub lessons: Vec<Lesson>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Lesson {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub steps: Vec<Step>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Step {
    /// markdown-lite: **bold**, *italic*, line breaks
    pub text: String,
    pub fen: Option<String>,
    pub orientation: Option<String>,
    pub arrows: Vec<Arrow>,
    /// squares
    pub highlights: Vec<String>,
    pub task: Option<Task>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Arrow {
    pub from: String,
    pub to: String,
    /// green | red | blue | yellow
    pub color: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Task {
    /// "moves" | "guess" | "count" | "hanging" | "choice" | "square" (see [`steps`])
    pub kind: String,
    pub prompt: String,
    /// UCI, alternating: user, reply, user, ... from step.fen
    pub solution: Vec<String>,
    pub hint: Option<String>,
    pub success: String,
    /// Fields of the interactive kinds (guess, count, hanging, choice, square): see [`steps`].
    #[serde(flatten)]
    pub extra: steps::TaskExtra,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct EndgameDrill {
    pub id: String,
    pub title: String,
    /// basic | pawn | rook | minor | queen
    pub category: String,
    pub level: String,
    pub fen: String,
    /// "win" | "draw"
    pub goal: String,
    pub description: String,
    pub hint: String,
    pub technique: Vec<String>,
    /// Extra starting positions for practice (same side to move and goal as `fen`).
    pub variants: Vec<String>,
    /// Short "key idea" lesson shown before practice (2–5 steps; `fen` defaults to the drill's).
    pub lesson: Vec<Step>,
    /// When a practice attempt counts as a success (see [`DrillSuccess`]).
    pub success: DrillSuccess,
    /// What usually goes wrong; shown after a failed attempt.
    pub pitfall: String,
}

/// Success condition of an endgame drill.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct DrillSuccess {
    /// `mate` | `promote` (safe promotion) | `bare_king` (opponent left with a lone king) |
    /// `hold` (draw goal: reach a drawn result or survive `moves` moves).
    pub kind: String,
    /// `hold`: own moves to survive; other kinds: move limit before the attempt fails.
    pub moves: u32,
}

/// Valid [`DrillSuccess::kind`] values.
pub const DRILL_SUCCESS_KINDS: [&str; 4] = ["mate", "promote", "bare_king", "hold"];

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct OpeningRef {
    pub id: String,
    pub eco: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct BookMove {
    pub uci: String,
    pub san: String,
    /// opening it leads to
    pub name: Option<String>,
    pub weight: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct OpeningMatch {
    pub opening: OpeningRef,
    pub continuations: Vec<BookMove>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Content {
    pub openings: Vec<Opening>,
    pub puzzles: Vec<Puzzle>,
    pub courses: Vec<Course>,
    pub endgames: Vec<EndgameDrill>,
    /// Annotated classic games (`classics.json`).
    pub classics: Vec<Classic>,
    /// Opening book index over `openings` (not serialized; see `rebuild_opening_index`).
    #[serde(skip)]
    pub opening_index: OpeningIndex,
    /// Language of the text in this instance (`En` for the source content).
    #[serde(skip)]
    pub lang: Lang,
    /// Per-language views (see [`Content::localized`]). Opaque; public only so that
    /// `Content { .., ..Default::default() }` works outside this crate.
    #[serde(skip)]
    pub views: LocalizedViews,
}

/// Shared table of per-language views, built once from the English source.
#[derive(Debug, Default)]
struct Slots {
    /// Overlay per language, in `Lang::ALL` order.
    overlays: Vec<Overlay>,
    /// Views in `Lang::ALL` order (built all at once so every view can reach every other).
    built: OnceLock<Vec<Arc<Content>>>,
}

/// Handle to the per-language views. The source content owns the table; views hold a weak
/// reference back to it, so there are no reference cycles. The default (hand-built content)
/// owns an empty table, so its views are built once on first use and then shared.
#[derive(Clone, Debug)]
pub struct LocalizedViews {
    own: Option<Arc<Slots>>,
    parent: Weak<Slots>,
}

impl Default for LocalizedViews {
    fn default() -> Self {
        LocalizedViews::with_overlays(Vec::new())
    }
}

impl LocalizedViews {
    fn with_overlays(overlays: Vec<Overlay>) -> Self {
        LocalizedViews { own: Some(Arc::new(Slots { overlays, built: OnceLock::new() })), parent: Weak::new() }
    }
}

// ---------------------------------------------------------------------------------------------
// Loading & validation
// ---------------------------------------------------------------------------------------------

/// Read a JSON array file; a missing file yields an empty vec.
fn read_list<T: DeserializeOwned>(path: &Path) -> anyhow::Result<Vec<T>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("content file {} not found; using empty list", path.display());
            return Ok(Vec::new());
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    // Deserialize element by element so one malformed entry doesn't drop the whole file.
    let values: Vec<serde_json::Value> =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {} (expected a JSON array)", path.display()))?;
    let mut out = Vec::with_capacity(values.len());
    for (i, v) in values.into_iter().enumerate() {
        match serde_json::from_value::<T>(v) {
            Ok(t) => out.push(t),
            Err(e) => tracing::warn!("{}: skipping entry #{i}: {e}", path.display()),
        }
    }
    Ok(out)
}

/// Replay SAN moves (space separated, move numbers like "1." / "1..." tolerated).
pub fn replay_san(start: &Chess, moves: &str) -> Result<(Chess, Vec<String>), String> {
    let mut pos = start.clone();
    let mut ucis = Vec::new();
    for tok in moves.split_whitespace() {
        // tolerate "1.e4" / "1." / "1..."
        let tok = tok.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches('.');
        if tok.is_empty() {
            continue;
        }
        let m = san_to_move(&pos, tok).map_err(|e| format!("move {}: {e}", ucis.len() + 1))?;
        ucis.push(move_to_uci(&m));
        pos.play_unchecked(&m);
    }
    Ok((pos, ucis))
}

/// Replay UCI moves from a position; returns the final position.
pub fn replay_uci(start: &Chess, moves: &[String]) -> Result<Chess, String> {
    let mut pos = start.clone();
    for (i, u) in moves.iter().enumerate() {
        let m = uci_to_move(&pos, u).map_err(|e| format!("ply {}: {e}", i + 1))?;
        pos.play_unchecked(&m);
    }
    Ok(pos)
}

fn validate_opening(o: &mut Opening) -> Result<(), String> {
    if o.id.is_empty() || o.name.is_empty() {
        return Err("missing id or name".into());
    }
    let (pos, uci) = replay_san(&Chess::default(), &o.moves)?;
    if uci.is_empty() {
        return Err("no moves".into());
    }
    o.uci = uci;
    o.fen = to_fen(&pos);
    Ok(())
}

fn validate_puzzle(p: &mut Puzzle) -> Result<(), String> {
    if p.id.is_empty() {
        return Err("missing id".into());
    }
    let start = parse_fen(&p.fen)?;
    if p.moves.len() < 2 {
        return Err("puzzle needs at least 2 moves (setup + solution)".into());
    }
    replay_uci(&start, &p.moves)?;
    p.fen = to_fen(&start);
    Ok(())
}

fn validate_course(c: &mut Course) -> Result<(), String> {
    if c.id.is_empty() || c.title.is_empty() {
        return Err("missing id or title".into());
    }
    let mut kept = Vec::with_capacity(c.lessons.len());
    for mut l in std::mem::take(&mut c.lessons) {
        match validate_lesson(&mut l) {
            Ok(()) => kept.push(l),
            Err(e) => tracing::warn!("course {}: skipping lesson {}: {e}", c.id, l.id),
        }
    }
    c.lessons = kept;
    if c.lessons.is_empty() {
        return Err("no valid lessons".into());
    }
    Ok(())
}

fn validate_lesson(l: &mut Lesson) -> Result<(), String> {
    if l.id.is_empty() {
        return Err("missing id".into());
    }
    for (i, s) in l.steps.iter_mut().enumerate() {
        validate_step(s).map_err(|e| format!("step {}: {e}", i + 1))?;
    }
    Ok(())
}

fn validate_step(s: &mut Step) -> Result<(), String> {
    let pos = match &s.fen {
        Some(f) => {
            let p = parse_fen(f)?;
            s.fen = Some(to_fen(&p));
            Some(p)
        }
        None => None,
    };
    if let Some(t) = &mut s.task {
        steps::validate_task(t, pos.as_ref())?;
    }
    Ok(())
}

fn is_square(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && (b'a'..=b'h').contains(&b[0]) && (b'1'..=b'8').contains(&b[1])
}

/// Most practice variants a drill may carry.
const MAX_DRILL_VARIANTS: usize = 8;

fn validate_endgame(d: &mut EndgameDrill) -> Result<(), String> {
    if d.id.is_empty() {
        return Err("missing id".into());
    }
    if d.goal != "win" && d.goal != "draw" {
        return Err(format!("bad goal {:?}", d.goal));
    }
    let p = parse_fen(&d.fen)?;
    if p.is_game_over() {
        return Err("position is already game over".into());
    }
    d.fen = to_fen(&p);
    if d.variants.len() > MAX_DRILL_VARIANTS {
        return Err("too many variants".into());
    }
    for v in d.variants.iter_mut() {
        let vp = parse_fen(v).map_err(|e| format!("variant {v:?}: {e}"))?;
        if vp.is_game_over() {
            return Err(format!("variant {v:?} is already game over"));
        }
        if vp.turn() != p.turn() {
            return Err(format!("variant {v:?}: different side to move"));
        }
        *v = to_fen(&vp);
    }
    // Defaults keep older drill files valid: mate for "win", hold for "draw".
    if d.success.kind.is_empty() {
        d.success.kind = if d.goal == "win" { "mate" } else { "hold" }.into();
    }
    if !DRILL_SUCCESS_KINDS.contains(&d.success.kind.as_str()) {
        return Err(format!("bad success kind {:?}", d.success.kind));
    }
    if (d.success.kind == "hold") != (d.goal == "draw") {
        return Err("success kind \"hold\" goes with goal \"draw\" (and only with it)".into());
    }
    if d.success.moves == 0 {
        d.success.moves = if d.goal == "win" { 50 } else { 30 };
    }
    if d.success.moves > 100 {
        return Err("success.moves must be at most 100".into());
    }
    if d.lesson.len() > 8 {
        return Err("lesson has too many steps".into());
    }
    for (i, s) in d.lesson.iter_mut().enumerate() {
        if s.task.is_some() {
            return Err(format!("lesson step {}: tasks are not supported", i + 1));
        }
        validate_step(s).map_err(|e| format!("lesson step {}: {e}", i + 1))?;
        let arrows = s.arrows.iter().flat_map(|a| [a.from.as_str(), a.to.as_str()]);
        if let Some(bad) = arrows.chain(s.highlights.iter().map(String::as_str)).find(|q| !is_square(q)) {
            return Err(format!("lesson step {}: bad square {bad:?}", i + 1));
        }
    }
    Ok(())
}

/// Keep entries that validate (and have unique ids); log and drop the rest.
fn retain_valid<T>(
    kind: &str,
    items: Vec<T>,
    id: impl Fn(&T) -> &str,
    mut validate: impl FnMut(&mut T) -> Result<(), String>,
) -> Vec<T> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for mut it in items {
        if let Err(e) = validate(&mut it) {
            tracing::warn!("{kind} {:?}: skipped: {e}", id(&it));
            continue;
        }
        if !seen.insert(id(&it).to_string()) {
            tracing::warn!("{kind} {:?}: skipped: duplicate id", id(&it));
            continue;
        }
        out.push(it);
    }
    out
}

impl Content {
    /// Loads `openings.json`, `puzzles.json`, `courses.json`, `endgames.json` from `dir`.
    /// Missing files yield empty lists. Invalid entries are skipped (and logged). Fills
    /// derived fields (`Opening.uci`, `Opening.fen`) and normalizes FENs.
    pub fn load(dir: &Path) -> anyhow::Result<Content> {
        let openings = retain_valid(
            "opening",
            read_list::<Opening>(&dir.join("openings.json"))?,
            |o| &o.id,
            validate_opening,
        );
        let puzzles = retain_valid(
            "puzzle",
            read_list::<Puzzle>(&dir.join("puzzles.json"))?,
            |p| &p.id,
            validate_puzzle,
        );
        let courses = retain_valid(
            "course",
            read_list::<Course>(&dir.join("courses.json"))?,
            |c| &c.id,
            validate_course,
        );
        let endgames = retain_valid(
            "endgame",
            read_list::<EndgameDrill>(&dir.join("endgames.json"))?,
            |d| &d.id,
            validate_endgame,
        );
        let classics = retain_valid(
            "classic",
            read_list::<Classic>(&dir.join("classics.json"))?,
            |g| &g.id,
            classics::validate_classic,
        );
        tracing::info!(
            "content loaded: {} openings, {} puzzles, {} courses, {} endgames, {} classics",
            openings.len(),
            puzzles.len(),
            courses.len(),
            endgames.len(),
            classics.len()
        );
        let opening_index = OpeningIndex::build(&openings);
        let i18n_dir = dir.join("i18n");
        let overlays = Lang::ALL
            .into_iter()
            .map(|lang| {
                let (ov, bad) = overlay::load_overlay(&i18n_dir, lang);
                if bad > 0 {
                    tracing::warn!("i18n/{lang}: {bad} overlay entries or files could not be parsed");
                }
                ov
            })
            .collect();
        let content = Content {
            openings,
            puzzles,
            courses,
            endgames,
            classics,
            opening_index,
            lang: Lang::En,
            views: LocalizedViews::with_overlays(overlays),
        };
        // Precompute every language view now (bounded: one view per supported language).
        let _ = content.localized(Lang::En);
        Ok(content)
    }

    /// Like [`Content::load`] but with overlays read from `i18n_dir` instead of `dir/i18n`.
    pub fn load_with_i18n(dir: &Path, i18n_dir: &Path) -> anyhow::Result<Content> {
        let mut c = Content::load(dir)?;
        let overlays = Lang::ALL.into_iter().map(|l| overlay::load_overlay(i18n_dir, l).0).collect();
        c.views = LocalizedViews::with_overlays(overlays);
        let _ = c.localized(Lang::En);
        Ok(c)
    }

    /// The content in `lang`: course/lesson/step text, opening names/descriptions/ideas/traps
    /// and endgame text replaced by the `data/i18n/<lang>/` overlays, falling back to English
    /// for anything missing. Moves, FENs, arrows and solutions are always the English source's.
    /// The opening book of the returned view reports localized names.
    ///
    /// Views are built once (at load for [`Content::load`]; on first use for hand-built
    /// content) and shared; calling this on a view returns the sibling view. Views reflect the
    /// source content at the time they were built ([`Content::rebuild_opening_index`] resets them).
    pub fn localized(&self, lang: Lang) -> Arc<Content> {
        let idx = Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0);
        let slots = match (&self.views.own, self.views.parent.upgrade()) {
            (Some(own), _) => Arc::clone(own),
            (None, Some(parent)) => parent,
            (None, None) => {
                // A view whose source was dropped: rebuild from this instance (rare; uncached).
                let own = Arc::new(Slots { overlays: Vec::new(), built: OnceLock::new() });
                let views = self.build_views(&own);
                return Arc::clone(&views[idx]);
            }
        };
        let views = slots.built.get_or_init(|| self.build_views(&slots));
        Arc::clone(&views[idx])
    }

    fn build_views(&self, slots: &Arc<Slots>) -> Vec<Arc<Content>> {
        let empty = Overlay::default();
        Lang::ALL
            .into_iter()
            .enumerate()
            .map(|(i, lang)| {
                let mut c = Content {
                    openings: self.openings.clone(),
                    puzzles: self.puzzles.clone(),
                    courses: self.courses.clone(),
                    endgames: self.endgames.clone(),
                    classics: self.classics.clone(),
                    opening_index: OpeningIndex::default(),
                    lang,
                    views: LocalizedViews { own: None, parent: Arc::downgrade(slots) },
                };
                let ov = slots.overlays.get(i).unwrap_or(&empty);
                if !ov.is_empty() {
                    let st = overlay::apply_overlay(&mut c, ov);
                    tracing::info!(
                        "i18n/{lang}: {} overlay entries applied, {} ignored (unknown ids or surplus steps)",
                        st.applied,
                        st.ignored
                    );
                }
                c.opening_index = OpeningIndex::build_lang(&c.openings, lang);
                Arc::new(c)
            })
            .collect()
    }

    /// Name data for an opening id in this content's language (also knows the start position).
    pub fn opening_ref(&self, id: &str) -> Option<OpeningRef> {
        if id == START_ID {
            return Some(OpeningRef { id: START_ID.to_string(), eco: String::new(), name: start_name(self.lang).to_string() });
        }
        self.opening(id).map(|o| OpeningRef { id: o.id.clone(), eco: o.eco.clone(), name: o.name.clone() })
    }

    /// Deepest named opening matching the position (key = first 4 FEN fields) + book continuations.
    ///
    /// Positions inside a line that are not themselves a named opening get the deepest named
    /// opening on a book path to them; the initial position reports "Starting Position" with
    /// all first moves. Transpositions are merged. Returns `None` for out-of-book positions or
    /// invalid FENs. O(1) hash lookup against an index built at load time.
    pub fn lookup_opening(&self, fen: &str) -> Option<OpeningMatch> {
        self.opening_book().lookup(fen)
    }

    /// True if the position occurs along any known opening line (including the start).
    pub fn is_book_position(&self, fen: &str) -> bool {
        self.opening_book().contains(fen)
    }

    /// The opening book index (built at load; built lazily for hand-constructed `Content`).
    pub fn opening_book(&self) -> &OpeningBook {
        self.opening_index.get(&self.openings)
    }

    /// Rebuild the opening index after mutating `openings` (the index is otherwise immutable).
    pub fn rebuild_opening_index(&mut self) {
        self.opening_index = OpeningIndex::build_lang(&self.openings, self.lang);
        let overlays = self.views.own.as_ref().map(|s| s.overlays.clone()).unwrap_or_default();
        self.views = LocalizedViews::with_overlays(overlays);
    }

    pub fn opening(&self, id: &str) -> Option<&Opening> {
        self.openings.iter().find(|o| o.id == id)
    }
    pub fn puzzle(&self, id: &str) -> Option<&Puzzle> {
        self.puzzles.iter().find(|p| p.id == id)
    }
    pub fn course(&self, id: &str) -> Option<&Course> {
        self.courses.iter().find(|c| c.id == id)
    }
    pub fn endgame(&self, id: &str) -> Option<&EndgameDrill> {
        self.endgames.iter().find(|d| d.id == id)
    }
    pub fn classic(&self, id: &str) -> Option<&Classic> {
        self.classics.iter().find(|g| g.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }

    fn raw<T: DeserializeOwned>(name: &str) -> Vec<T> {
        let bytes = std::fs::read(data_dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// Every entry in every data file must parse and be legal — nothing may be silently dropped.
    #[test]
    fn all_data_is_legal() {
        let c = Content::load(&data_dir()).expect("load content");

        let openings: Vec<Opening> = raw("openings.json");
        assert_eq!(c.openings.len(), openings.len(), "some openings were invalid or duplicated");
        for o in &openings {
            let mut o = o.clone();
            validate_opening(&mut o).unwrap_or_else(|e| panic!("opening {}: {e}", o.id));
        }

        let puzzles: Vec<Puzzle> = raw("puzzles.json");
        assert_eq!(c.puzzles.len(), puzzles.len(), "some puzzles were invalid or duplicated");
        for p in &puzzles {
            let start = parse_fen(&p.fen).unwrap_or_else(|e| panic!("puzzle {}: {e}", p.id));
            assert!(p.moves.len() >= 2, "puzzle {}: too few moves", p.id);
            replay_uci(&start, &p.moves).unwrap_or_else(|e| panic!("puzzle {}: {e}", p.id));
        }

        let courses: Vec<Course> = raw("courses.json");
        assert_eq!(c.courses.len(), courses.len(), "some courses were invalid or duplicated");
        for course in &courses {
            for l in &course.lessons {
                for (i, s) in l.steps.iter().enumerate() {
                    let mut s = s.clone();
                    validate_step(&mut s)
                        .unwrap_or_else(|e| panic!("course {} lesson {} step {}: {e}", course.id, l.id, i + 1));
                }
            }
            let loaded = c.course(&course.id).expect("course loaded");
            assert_eq!(loaded.lessons.len(), course.lessons.len(), "course {}: lessons dropped", course.id);
        }

        let endgames: Vec<EndgameDrill> = raw("endgames.json");
        assert_eq!(c.endgames.len(), endgames.len(), "some endgames were invalid or duplicated");
        for d in &endgames {
            let mut v = d.clone();
            validate_endgame(&mut v).unwrap_or_else(|e| panic!("endgame {}: {e}", d.id));
            // Theory drills: each ships a short lesson, a pitfall, practice variants and an
            // explicit success condition.
            assert!((2..=5).contains(&d.lesson.len()), "endgame {}: lesson needs 2-5 steps", d.id);
            assert!(!d.pitfall.is_empty(), "endgame {}: missing pitfall", d.id);
            assert!(!d.variants.is_empty(), "endgame {}: needs at least one variant", d.id);
            assert!(!d.success.kind.is_empty() && d.success.moves > 0, "endgame {}: explicit success", d.id);
            let mut fens: Vec<&str> = std::iter::once(d.fen.as_str()).chain(d.variants.iter().map(String::as_str)).collect();
            fens.sort_unstable();
            fens.dedup();
            assert_eq!(fens.len(), d.variants.len() + 1, "endgame {}: duplicate variant", d.id);
        }
        for cat in ["basic", "pawn", "rook", "minor", "queen"] {
            assert!(c.endgames.iter().any(|d| d.category == cat), "no {cat} endgames");
        }
    }

    #[test]
    fn endgame_validation_rejects_bad_drills() {
        let base = EndgameDrill {
            id: "x".into(),
            goal: "win".into(),
            fen: "4k3/8/8/8/8/8/8/R3K3 w - - 0 1".into(),
            ..Default::default()
        };
        let mut ok = base.clone();
        validate_endgame(&mut ok).unwrap();
        assert_eq!(ok.success, DrillSuccess { kind: "mate".into(), moves: 50 });

        let mut bad = base.clone();
        bad.variants = vec!["4k3/8/8/8/8/8/8/R3K3 b - - 0 1".into()];
        assert!(validate_endgame(&mut bad).is_err(), "variant with the other side to move");
        let mut bad = base.clone();
        bad.success.kind = "hold".into();
        assert!(validate_endgame(&mut bad).is_err(), "hold on a win drill");
        let mut bad = base.clone();
        bad.success.kind = "fly".into();
        assert!(validate_endgame(&mut bad).is_err());
        let mut bad = base.clone();
        bad.lesson = vec![Step {
            text: "x".into(),
            arrows: vec![Arrow { from: "a1".into(), to: "i9".into(), color: "green".into() }],
            ..Default::default()
        }];
        assert!(validate_endgame(&mut bad).is_err(), "bad arrow square");
        let mut bad = base;
        bad.goal = "lose".into();
        assert!(validate_endgame(&mut bad).is_err());
    }

    #[test]
    fn missing_dir_is_empty() {
        let c = Content::load(Path::new("/definitely/not/here")).unwrap();
        assert!(c.openings.is_empty() && c.puzzles.is_empty() && c.courses.is_empty() && c.endgames.is_empty());
    }

    #[test]
    fn opening_fields_filled() {
        let c = Content::load(&data_dir()).unwrap();
        for o in &c.openings {
            assert!(!o.uci.is_empty() && !o.fen.is_empty(), "{}", o.id);
        }
        assert!(c.is_book_position(gm_engine::START_FEN));
    }

    #[test]
    fn bad_entries_skipped() {
        let mut o = Opening { id: "x".into(), name: "X".into(), moves: "e4 e5 Ke3".into(), ..Default::default() };
        assert!(validate_opening(&mut o).is_err());
        let kept = retain_valid("opening", vec![o], |o| &o.id, validate_opening);
        assert!(kept.is_empty());
    }
}
