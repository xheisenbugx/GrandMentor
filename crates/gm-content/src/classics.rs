//! Annotated classic games (`data/classics.json`) and their translation overlay
//! (`data/i18n/<lang>/classics*.json`).
//!
//! Every game is replayed at load time: the SAN score must be legal from the initial position,
//! annotation/question plies must point inside the game and question alternatives must be legal.
//! The loader fills `uci`, `san`, `plies`, `key_fen`, `era` and each question's `answer_*`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Position};

use gm_engine::{move_to_san, move_to_uci, san_to_move, to_fen};

use crate::overlay::{set_text, OverlayStats};
use crate::Arrow;

/// Upper bounds (content is trusted but still bounded).
const MAX_PLIES: usize = 600;
const MAX_NOTES: usize = 64;
const MAX_QUESTIONS: usize = 8;
const MAX_THEMES: usize = 8;

pub const LEVELS: &[&str] = &["beginner", "intermediate", "advanced"];
pub const RESULTS: &[&str] = &["1-0", "0-1", "1/2-1/2"];
/// Known theme slugs (the frontend translates them).
pub const THEMES: &[&str] = &[
    "development", "attack", "sacrifice", "king-hunt", "checkmate", "tactics", "positional", "endgame", "defence",
    "opening-trap", "back-rank", "zugzwang", "initiative", "pawn-power", "human-vs-machine", "calculation",
];

/// A mentor comment shown right after ply `ply` (0 = before the first move).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ClassicNote {
    pub ply: usize,
    pub text: String,
    /// Present on "key moments" (short chip text).
    pub label: Option<String>,
    pub arrows: Vec<Arrow>,
    pub highlights: Vec<String>,
}

/// "Pause and think": the board stops after `ply - 1` plies; the answer is the game's `ply`-th move.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ClassicQuestion {
    pub ply: usize,
    pub prompt: String,
    pub hint: Option<String>,
    pub explanation: String,
    /// Other moves (SAN in the source) that also count as correct.
    pub also: Vec<String>,
    /// filled by loader: the game move (UCI / SAN).
    pub answer_uci: String,
    pub answer_san: String,
    /// filled by loader: every accepted move in UCI (answer first).
    pub accept: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Classic {
    pub id: String,
    pub title: String,
    pub white: String,
    pub black: String,
    pub event: String,
    pub year: i32,
    /// "1-0" | "0-1" | "1/2-1/2"
    pub result: String,
    pub opening: String,
    /// beginner | intermediate | advanced
    pub level: String,
    pub themes: Vec<String>,
    /// "white" | "black": the side the learner watches from.
    pub orientation: String,
    pub summary: String,
    /// SAN source score, space separated.
    pub moves: String,
    /// Critical position shown on the library card (position after `key_ply` plies).
    pub key_ply: usize,
    pub annotations: Vec<ClassicNote>,
    pub questions: Vec<ClassicQuestion>,
    /// filled by loader
    pub uci: Vec<String>,
    /// filled by loader (normalized SAN with check suffixes)
    pub san: Vec<String>,
    /// filled by loader
    pub plies: usize,
    /// filled by loader: FEN after `key_ply` plies
    pub key_fen: String,
    /// filled by loader from `year`: romantic | classical | modern | computer
    pub era: String,
}

/// List entry for `GET /api/classics` (no moves / annotations).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ClassicSummary {
    pub id: String,
    pub title: String,
    pub white: String,
    pub black: String,
    pub event: String,
    pub year: i32,
    pub result: String,
    pub opening: String,
    pub level: String,
    pub themes: Vec<String>,
    pub era: String,
    pub orientation: String,
    pub summary: String,
    pub key_fen: String,
    pub plies: usize,
    pub annotation_count: usize,
    pub question_count: usize,
}

impl Classic {
    pub fn summary(&self) -> ClassicSummary {
        ClassicSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            white: self.white.clone(),
            black: self.black.clone(),
            event: self.event.clone(),
            year: self.year,
            result: self.result.clone(),
            opening: self.opening.clone(),
            level: self.level.clone(),
            themes: self.themes.clone(),
            era: self.era.clone(),
            orientation: self.orientation.clone(),
            summary: self.summary.clone(),
            key_fen: self.key_fen.clone(),
            plies: self.plies,
            annotation_count: self.annotations.len(),
            question_count: self.questions.len(),
        }
    }
}

/// Era bucket for the library filter.
pub fn era_of(year: i32) -> &'static str {
    match year {
        ..=1885 => "romantic",
        1886..=1945 => "classical",
        1946..=1990 => "modern",
        _ => "computer",
    }
}

fn is_square(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && (b'a'..=b'h').contains(&b[0]) && (b'1'..=b'8').contains(&b[1])
}

/// Validate a game and fill the derived fields.
pub(crate) fn validate_classic(g: &mut Classic) -> Result<(), String> {
    if g.id.is_empty() || g.id.len() > 80 || !g.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err("missing or invalid id".into());
    }
    if g.title.trim().is_empty() || g.white.trim().is_empty() || g.black.trim().is_empty() {
        return Err("missing title or players".into());
    }
    if !LEVELS.contains(&g.level.as_str()) {
        return Err(format!("unknown level {:?}", g.level));
    }
    if !RESULTS.contains(&g.result.as_str()) {
        return Err(format!("unknown result {:?}", g.result));
    }
    if g.themes.is_empty() || g.themes.len() > MAX_THEMES || g.themes.iter().any(|t| !THEMES.contains(&t.as_str())) {
        return Err(format!("bad themes {:?}", g.themes));
    }
    if g.orientation.is_empty() {
        g.orientation = "white".into();
    }
    if g.orientation != "white" && g.orientation != "black" {
        return Err(format!("bad orientation {:?}", g.orientation));
    }

    // Replay the score, remembering every position (before each ply).
    let mut pos = Chess::default();
    let mut positions = vec![pos.clone()];
    let (mut uci, mut san) = (Vec::new(), Vec::new());
    for tok in g.moves.split_whitespace() {
        let tok = tok.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches('.');
        if tok.is_empty() {
            continue;
        }
        if uci.len() >= MAX_PLIES {
            return Err(format!("more than {MAX_PLIES} plies"));
        }
        let m = san_to_move(&pos, tok).map_err(|e| format!("ply {}: {e}", uci.len() + 1))?;
        san.push(move_to_san(&pos, &m));
        uci.push(move_to_uci(&m));
        pos.play_unchecked(&m);
        positions.push(pos.clone());
    }
    let plies = uci.len();
    if plies == 0 {
        return Err("no moves".into());
    }
    if g.key_ply > plies {
        return Err(format!("key_ply {} beyond the game ({plies} plies)", g.key_ply));
    }

    if g.annotations.len() > MAX_NOTES {
        return Err("too many annotations".into());
    }
    g.annotations.sort_by_key(|n| n.ply);
    for w in g.annotations.windows(2) {
        if w[0].ply == w[1].ply {
            return Err(format!("two annotations on ply {}", w[0].ply));
        }
    }
    for n in &g.annotations {
        if n.ply > plies {
            return Err(format!("annotation on ply {} beyond the game", n.ply));
        }
        if n.text.trim().is_empty() {
            return Err(format!("empty annotation on ply {}", n.ply));
        }
        if n.arrows.iter().any(|a| !is_square(&a.from) || !is_square(&a.to)) || n.highlights.iter().any(|s| !is_square(s)) {
            return Err(format!("bad arrow or highlight on ply {}", n.ply));
        }
    }

    if g.questions.len() > MAX_QUESTIONS {
        return Err("too many questions".into());
    }
    g.questions.sort_by_key(|q| q.ply);
    for q in &mut g.questions {
        if q.ply == 0 || q.ply > plies {
            return Err(format!("question on ply {} outside the game", q.ply));
        }
        if q.prompt.trim().is_empty() || q.explanation.trim().is_empty() {
            return Err(format!("question on ply {} needs a prompt and an explanation", q.ply));
        }
        let before = &positions[q.ply - 1];
        q.answer_uci = uci[q.ply - 1].clone();
        q.answer_san = san[q.ply - 1].clone();
        q.accept = vec![q.answer_uci.clone()];
        for alt in &q.also {
            let m = san_to_move(before, alt).map_err(|e| format!("question on ply {}: alternative {e}", q.ply))?;
            let u = move_to_uci(&m);
            if !q.accept.contains(&u) {
                q.accept.push(u);
            }
        }
    }
    if g.questions.windows(2).any(|w| w[0].ply == w[1].ply) {
        return Err("two questions on the same ply".into());
    }

    g.key_fen = to_fen(&positions[g.key_ply]);
    g.era = era_of(g.year).to_string();
    g.plies = plies;
    g.uci = uci;
    g.san = san;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Translation overlay
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct NoteOverlay {
    pub text: Option<String>,
    pub label: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct QuestionOverlay {
    pub prompt: Option<String>,
    pub hint: Option<String>,
    pub explanation: Option<String>,
}

/// `data/i18n/<lang>/classics*.json`: `{ "<id>": ClassicOverlay }`. Annotation/question lists
/// have the same order and length as the English source (`null` = untranslated entry).
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ClassicOverlay {
    pub title: Option<String>,
    pub event: Option<String>,
    pub opening: Option<String>,
    pub summary: Option<String>,
    pub annotations: Vec<Option<NoteOverlay>>,
    pub questions: Vec<Option<QuestionOverlay>>,
}

/// Apply translated text (never moves) to `games`.
pub(crate) fn apply_overlay(games: &mut [Classic], ov: &BTreeMap<String, ClassicOverlay>) -> OverlayStats {
    let mut st = OverlayStats::default();
    for (id, co) in ov {
        let Some(g) = games.iter_mut().find(|g| &g.id == id) else {
            st.ignored += 1;
            continue;
        };
        let mut any = set_text(&mut g.title, &co.title);
        any |= set_text(&mut g.event, &co.event);
        any |= set_text(&mut g.opening, &co.opening);
        any |= set_text(&mut g.summary, &co.summary);
        st.ignored += co.annotations.len().saturating_sub(g.annotations.len());
        st.ignored += co.questions.len().saturating_sub(g.questions.len());
        for (n, no) in g.annotations.iter_mut().zip(&co.annotations) {
            let Some(no) = no else { continue };
            any |= set_text(&mut n.text, &no.text);
            if let Some(l) = n.label.as_mut() {
                any |= set_text(l, &no.label);
            }
        }
        for (q, qo) in g.questions.iter_mut().zip(&co.questions) {
            let Some(qo) = qo else { continue };
            any |= set_text(&mut q.prompt, &qo.prompt);
            any |= set_text(&mut q.explanation, &qo.explanation);
            if let Some(h) = q.hint.as_mut() {
                any |= set_text(h, &qo.hint);
            }
        }
        if any {
            st.applied += 1;
        }
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Content, Lang};
    use std::path::PathBuf;

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }

    fn game(moves: &str) -> Classic {
        Classic {
            id: "t".into(),
            title: "T".into(),
            white: "W".into(),
            black: "B".into(),
            year: 1858,
            result: "1-0".into(),
            level: "beginner".into(),
            themes: vec!["attack".into()],
            moves: moves.into(),
            key_ply: 2,
            annotations: vec![ClassicNote { ply: 1, text: "x".into(), ..Default::default() }],
            questions: vec![ClassicQuestion {
                ply: 3,
                prompt: "?".into(),
                explanation: "!".into(),
                also: vec!["Bc4".into()],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn validates_and_fills() {
        let mut g = game("e4 e5 Nf3 Nc6");
        validate_classic(&mut g).unwrap();
        assert_eq!(g.plies, 4);
        assert_eq!(g.uci, ["e2e4", "e7e5", "g1f3", "b8c6"]);
        assert_eq!(g.era, "romantic");
        assert_eq!(g.orientation, "white");
        assert!(g.key_fen.starts_with("rnbqkbnr/pppp1ppp/8/4p3/4P3/"));
        assert_eq!(g.questions[0].answer_san, "Nf3");
        assert_eq!(g.questions[0].accept, ["g1f3", "f1c4"]);
    }

    #[test]
    fn rejects_bad_games() {
        assert!(validate_classic(&mut game("e4 e5 Ke3")).is_err());
        let mut g = game("e4 e5 Nf3 Nc6");
        g.annotations[0].ply = 9;
        assert!(validate_classic(&mut g).is_err());
        let mut g = game("e4 e5 Nf3 Nc6");
        g.questions[0].also = vec!["Qh8".into()];
        assert!(validate_classic(&mut g).is_err());
        let mut g = game("e4 e5 Nf3 Nc6");
        g.themes = vec!["nope".into()];
        assert!(validate_classic(&mut g).is_err());
        let mut g = game("e4 e5 Nf3 Nc6");
        g.key_ply = 5;
        assert!(validate_classic(&mut g).is_err());
    }

    #[test]
    fn eras() {
        assert_eq!(era_of(1851), "romantic");
        assert_eq!(era_of(1938), "classical");
        assert_eq!(era_of(1972), "modern");
        assert_eq!(era_of(1997), "computer");
    }

    /// Every game in `data/classics.json` must load (no entry dropped), and the Spanish overlay
    /// must translate every game completely with matching annotation/question counts.
    #[test]
    fn all_classics_are_legal_and_translated() {
        let raw: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(data_dir().join("classics.json")).expect("classics.json")).expect("json");
        let c = Content::load(&data_dir()).expect("load");
        assert_eq!(c.classics.len(), raw.len(), "some classic games were invalid or duplicated");
        assert!(c.classics.len() >= 24, "expected at least 24 classic games");
        for g in &c.classics {
            assert!(!g.annotations.is_empty() && !g.questions.is_empty(), "{}", g.id);
        }

        let es: BTreeMap<String, ClassicOverlay> = serde_json::from_slice(
            &std::fs::read(data_dir().join("i18n/es/classics.json")).expect("es/classics.json"),
        )
        .expect("es json");
        let view = c.localized(Lang::Es);
        for g in &c.classics {
            let o = es.get(&g.id).unwrap_or_else(|| panic!("es: missing {}", g.id));
            assert_eq!(o.annotations.len(), g.annotations.len(), "es {}: annotations", g.id);
            assert_eq!(o.questions.len(), g.questions.len(), "es {}: questions", g.id);
            let v = view.classic(&g.id).expect("view");
            assert_ne!(v.summary, g.summary, "es {}: summary untranslated", g.id);
            assert_eq!(v.uci, g.uci, "overlay must not change moves");
        }
        assert!(es.keys().all(|k| c.classic(k).is_some()), "es overlay has unknown ids");
    }

    #[test]
    fn overlay_replaces_text_only() {
        let mut g = game("e4 e5 Nf3 Nc6");
        g.annotations[0].label = Some("Key".into());
        validate_classic(&mut g).unwrap();
        let mut games = vec![g.clone()];
        let mut ov = BTreeMap::new();
        ov.insert(
            "t".to_string(),
            ClassicOverlay {
                title: Some("Título".into()),
                annotations: vec![Some(NoteOverlay { text: Some("hola".into()), label: Some("Clave".into()) }), None],
                questions: vec![Some(QuestionOverlay { prompt: Some("¿?".into()), hint: Some("pista".into()), ..Default::default() })],
                ..Default::default()
            },
        );
        ov.insert("missing".to_string(), ClassicOverlay::default());
        let st = apply_overlay(&mut games, &ov);
        assert_eq!(st.applied, 1);
        assert_eq!(st.ignored, 2); // unknown id + surplus annotation
        assert_eq!(games[0].title, "Título");
        assert_eq!(games[0].annotations[0].text, "hola");
        assert_eq!(games[0].annotations[0].label.as_deref(), Some("Clave"));
        assert_eq!(games[0].questions[0].prompt, "¿?");
        assert_eq!(games[0].questions[0].hint, None); // no English hint -> none added
        assert_eq!(games[0].uci, g.uci);
    }
}
