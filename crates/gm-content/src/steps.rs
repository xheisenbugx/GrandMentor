//! Interactive lesson task kinds (`Task.kind`) and their validation.
//!
//! | kind      | learner does…                                   | fields used                         |
//! |-----------|-------------------------------------------------|-------------------------------------|
//! | `moves`   | plays the solution (replies are auto-played)    | `solution`                          |
//! | `guess`   | guesses a master's moves (scored, then revealed)| `solution`, `notes`, `game`         |
//! | `count`   | says who is ahead in material and by how much   | `answer`, `choices`                 |
//! | `hanging` | clicks every undefended / en-prise piece        | `squares`                           |
//! | `choice`  | picks the best plan among options (with arrows) | `options`                           |
//! | `square`  | clicks the named squares (coordinate quiz)      | `squares`, `blind`                  |
//!
//! Every field is checked against the position, so a broken lesson is dropped at load time and
//! `cargo test -p gm-content` fails. Schema: `docs/CONTRACT.md` ("Interactive lessons").

use serde::{Deserialize, Serialize};
use shakmaty::{Board, Chess, Color, Position, Role, Square};

use crate::{replay_uci, Arrow, Task};

/// Bounds on authored data.
const MAX_SQUARES: usize = 16;
const MAX_OPTIONS: usize = 6;
const MAX_CHOICES: usize = 7;
const ARROW_COLORS: [&str; 4] = ["green", "red", "blue", "yellow"];

/// The task kinds the lesson player understands.
pub const TASK_KINDS: [&str; 6] = ["moves", "guess", "count", "hanging", "choice", "square"];

/// Extra task fields used by the non-`moves` kinds (flattened into [`Task`] on the wire; empty
/// fields are omitted so `moves` tasks serialize exactly as before).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct TaskExtra {
    /// `guess`: the game, e.g. "Paul Morphy vs Duke Karl & Count Isouard, Paris 1858".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    /// `guess`: one short explanation per learner move (`solution[0]`, `solution[2]`, …).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// `count`: material balance in pawns, White minus Black (1/3/3/5/9). Checked against the FEN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<i32>,
    /// `count`: the answer buttons (same unit as `answer`, which must be one of them).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<i32>,
    /// `hanging`: every hanging piece (checked: no other piece of those colours is hanging).
    /// `square`: the squares to click, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub squares: Vec<String>,
    /// `square`: hide the board coordinates while the quiz runs.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub blind: bool,
    /// `choice`: the options (at least one `correct`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ChoiceOption>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ChoiceOption {
    pub text: String,
    /// Arrows drawn on the board while this option is hovered / selected.
    pub arrows: Vec<Arrow>,
    pub correct: bool,
    /// Why this option is right / wrong (shown after the answer).
    pub explain: String,
}

/// Point value of a piece (kings count 0).
pub fn piece_value(role: Role) -> i32 {
    match role {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 0,
    }
}

/// Material balance in pawns, White minus Black.
pub fn material_balance(board: &Board) -> i32 {
    let side = |c: Color| -> i32 {
        let m = board.material_side(c);
        i32::from(m.pawn) + 3 * i32::from(m.knight) + 3 * i32::from(m.bishop) + 5 * i32::from(m.rook) + 9 * i32::from(m.queen)
    };
    side(Color::White) - side(Color::Black)
}

/// A non-king piece that is attacked and either undefended or attacked by a cheaper piece.
pub fn is_hanging(board: &Board, sq: Square) -> bool {
    let Some(piece) = board.piece_at(sq) else { return false };
    if piece.role == Role::King {
        return false;
    }
    let occ = board.occupied();
    let attackers = board.attacks_to(sq, !piece.color, occ);
    if attackers.is_empty() {
        return false;
    }
    let defenders = board.attacks_to(sq, piece.color, occ);
    if defenders.is_empty() {
        return true;
    }
    let value = piece_value(piece.role);
    attackers
        .into_iter()
        .filter_map(|a| board.piece_at(a))
        .any(|a| a.role != Role::King && piece_value(a.role) < value)
}

/// All hanging pieces of `color`.
pub fn hanging_pieces(board: &Board, color: Color) -> Vec<Square> {
    board.by_color(color).into_iter().filter(|&sq| is_hanging(board, sq)).collect()
}

fn parse_square(s: &str) -> Result<Square, String> {
    let ok = s.len() == 2 && matches!(s.as_bytes()[0], b'a'..=b'h') && matches!(s.as_bytes()[1], b'1'..=b'8');
    if !ok {
        return Err(format!("bad square {s:?}"));
    }
    s.parse::<Square>().map_err(|_| format!("bad square {s:?}"))
}

fn check_arrows(arrows: &[Arrow]) -> Result<(), String> {
    for a in arrows {
        parse_square(&a.from)?;
        parse_square(&a.to)?;
        if !a.color.is_empty() && !ARROW_COLORS.contains(&a.color.as_str()) {
            return Err(format!("bad arrow colour {:?}", a.color));
        }
    }
    Ok(())
}

/// Validate a task against the step position (`None` when the step has no FEN of its own).
/// Normalizes `kind` (empty → `moves`) and lower-cases squares.
pub(crate) fn validate_task(t: &mut Task, pos: Option<&Chess>) -> Result<(), String> {
    if t.kind.trim().is_empty() {
        t.kind = "moves".into();
    }
    let kind = t.kind.trim().to_ascii_lowercase();
    if !TASK_KINDS.contains(&kind.as_str()) {
        return Err(format!("unknown task kind {:?}", t.kind));
    }
    t.kind = kind;
    for sq in &mut t.extra.squares {
        *sq = sq.trim().to_ascii_lowercase();
    }
    let need_pos = || pos.ok_or_else(|| "task without fen".to_string());
    match t.kind.as_str() {
        "moves" | "guess" => {
            let p = need_pos()?;
            if t.solution.is_empty() {
                return Err("task without solution".into());
            }
            replay_uci(p, &t.solution)?;
            if t.kind == "guess" {
                let learner_moves = t.solution.len().div_ceil(2);
                if t.extra.notes.len() > learner_moves {
                    return Err(format!("{} notes for {learner_moves} learner moves", t.extra.notes.len()));
                }
            }
        }
        "count" => {
            let p = need_pos()?;
            let actual = material_balance(p.board());
            let answer = t.extra.answer.ok_or("count task without answer")?;
            if answer != actual {
                return Err(format!("count answer {answer} but the position is {actual}"));
            }
            let c = &t.extra.choices;
            if c.len() < 2 || c.len() > MAX_CHOICES {
                return Err(format!("count task needs 2..={MAX_CHOICES} choices"));
            }
            if !c.contains(&answer) {
                return Err("answer is not one of the choices".into());
            }
            let mut uniq = c.clone();
            uniq.sort_unstable();
            uniq.dedup();
            if uniq.len() != c.len() {
                return Err("duplicate choices".into());
            }
        }
        "hanging" => {
            let p = need_pos()?;
            let board = p.board();
            if t.extra.squares.is_empty() || t.extra.squares.len() > MAX_SQUARES {
                return Err("hanging task needs 1.. squares".into());
            }
            let mut colors = Vec::new();
            let mut listed = Vec::new();
            for s in &t.extra.squares {
                let sq = parse_square(s)?;
                if !is_hanging(board, sq) {
                    return Err(format!("{s} is not a hanging piece"));
                }
                if let Some(pc) = board.piece_at(sq) {
                    if !colors.contains(&pc.color) {
                        colors.push(pc.color);
                    }
                }
                listed.push(sq);
            }
            for c in colors {
                if let Some(extra) = hanging_pieces(board, c).into_iter().find(|sq| !listed.contains(sq)) {
                    return Err(format!("{extra} is also hanging but not listed"));
                }
            }
        }
        "choice" => {
            let o = &t.extra.options;
            if o.len() < 2 || o.len() > MAX_OPTIONS {
                return Err(format!("choice task needs 2..={MAX_OPTIONS} options"));
            }
            if !o.iter().any(|x| x.correct) {
                return Err("choice task has no correct option".into());
            }
            for x in o {
                if x.text.trim().is_empty() {
                    return Err("choice option without text".into());
                }
                check_arrows(&x.arrows)?;
            }
        }
        "square" => {
            if t.extra.squares.is_empty() || t.extra.squares.len() > MAX_SQUARES {
                return Err(format!("square task needs 1..={MAX_SQUARES} squares"));
            }
            for s in &t.extra.squares {
                parse_square(s)?;
            }
        }
        _ => unreachable!("kind checked above"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gm_engine::parse_fen;

    fn task(kind: &str) -> Task {
        Task { kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn material_and_counting() {
        // White: R + 3P = 8, Black: B + 3P = 6
        let p = parse_fen("6k1/5ppp/8/8/8/2b5/5PPP/3R2K1 w - - 0 1").unwrap();
        assert_eq!(material_balance(p.board()), 2);
        let mut t = task("count");
        t.extra.answer = Some(2);
        t.extra.choices = vec![-2, 0, 2];
        assert!(validate_task(&mut t, Some(&p)).is_ok());
        t.extra.answer = Some(1);
        t.extra.choices = vec![1, 2];
        assert!(validate_task(&mut t, Some(&p)).unwrap_err().contains("position is 2"));
        t.extra.answer = Some(2);
        t.extra.choices = vec![0, 1];
        assert!(validate_task(&mut t, Some(&p)).is_err());
        assert!(validate_task(&mut task("count"), None).is_err());
    }

    #[test]
    fn hanging_detection() {
        // Black knight on c6 attacked by Bb5, undefended; white pawn e4 defended by d3.
        let p = parse_fen("4k3/8/2n5/1B6/4P3/3P4/8/4K3 b - - 0 1").unwrap();
        let mut t = task("hanging");
        t.extra.squares = vec!["C6".into()];
        assert!(validate_task(&mut t, Some(&p)).is_ok(), "{:?}", validate_task(&mut t.clone(), Some(&p)));
        assert_eq!(t.extra.squares, vec!["c6"]);
        t.extra.squares = vec!["e4".into()];
        assert!(validate_task(&mut t, Some(&p)).is_err(), "e4 is not attacked");
        // A queen attacked by a pawn is hanging even when defended.
        let q = parse_fen("4k3/8/8/3q4/4P3/8/8/3RK3 b - - 0 1").unwrap();
        assert!(is_hanging(q.board(), Square::D5));
        // Listing only part of the hanging pieces is an error.
        let two = parse_fen("4k3/8/2n1n3/1B1B4/8/8/8/4K3 b - - 0 1").unwrap();
        let mut t = task("hanging");
        t.extra.squares = vec!["c6".into()];
        assert!(validate_task(&mut t, Some(&two)).unwrap_err().contains("also hanging"));
        t.extra.squares = vec!["c6".into(), "e6".into()];
        assert!(validate_task(&mut t, Some(&two)).is_ok());
    }

    #[test]
    fn choice_square_guess_and_kinds() {
        let mut t = task("choice");
        t.extra.options = vec![
            ChoiceOption { text: "A".into(), correct: true, arrows: vec![Arrow { from: "e2".into(), to: "e4".into(), color: "green".into() }], ..Default::default() },
            ChoiceOption { text: "B".into(), ..Default::default() },
        ];
        assert!(validate_task(&mut t, None).is_ok());
        t.extra.options[0].correct = false;
        assert!(validate_task(&mut t, None).is_err());
        t.extra.options[0].correct = true;
        t.extra.options[1].arrows = vec![Arrow { from: "e9".into(), to: "e4".into(), color: String::new() }];
        assert!(validate_task(&mut t, None).is_err());

        let mut t = task("square");
        t.extra.squares = vec!["e4".into(), "h8".into()];
        assert!(validate_task(&mut t, None).is_ok());
        t.extra.squares = vec!["i4".into()];
        assert!(validate_task(&mut t, None).is_err());

        let start = parse_fen(gm_engine::START_FEN).unwrap();
        let mut t = task("guess");
        t.solution = vec!["e2e4".into(), "e7e5".into(), "g1f3".into()];
        t.extra.notes = vec!["a".into(), "b".into()];
        assert!(validate_task(&mut t, Some(&start)).is_ok());
        t.extra.notes.push("c".into());
        assert!(validate_task(&mut t, Some(&start)).is_err());

        let mut t = task("");
        t.solution = vec!["e2e4".into()];
        assert!(validate_task(&mut t, Some(&start)).is_ok());
        assert_eq!(t.kind, "moves");
        assert!(validate_task(&mut task("dance"), Some(&start)).is_err());
    }

    /// The interactive courses ship with every kind and are fully translated to Spanish
    /// (step text, prompts, success messages, guess notes, game captions and choice options).
    #[test]
    fn interactive_courses_load_and_are_translated() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let en = crate::Content::load(&dir).expect("load");
        let es = en.localized(crate::Lang::Es);
        let mut kinds = std::collections::BTreeSet::new();
        for id in ["board-vision", "guess-the-move"] {
            let c_en = en.course(id).unwrap_or_else(|| panic!("{id} missing"));
            let c_es = es.course(id).expect("es course");
            assert_ne!(c_en.title, c_es.title, "{id}: title");
            assert!(c_en.lessons.len() >= 5, "{id}: lessons");
            for (l_en, l_es) in c_en.lessons.iter().zip(&c_es.lessons) {
                assert_ne!(l_en.title, l_es.title, "{id}/{}: title", l_en.id);
                for (i, (s_en, s_es)) in l_en.steps.iter().zip(&l_es.steps).enumerate() {
                    let at = format!("{id}/{} step {}", l_en.id, i + 1);
                    assert_ne!(s_en.text, s_es.text, "{at}: text");
                    let (Some(t_en), Some(t_es)) = (&s_en.task, &s_es.task) else { continue };
                    kinds.insert(t_en.kind.clone());
                    assert_ne!(t_en.prompt, t_es.prompt, "{at}: prompt");
                    assert_ne!(t_en.success, t_es.success, "{at}: success");
                    if t_en.hint.is_some() {
                        assert_ne!(t_en.hint, t_es.hint, "{at}: hint");
                    }
                    if t_en.extra.game.is_some() {
                        assert_ne!(t_en.extra.game, t_es.extra.game, "{at}: game");
                    }
                    for (a, b) in t_en.extra.notes.iter().zip(&t_es.extra.notes) {
                        assert_ne!(a, b, "{at}: note");
                    }
                    for (a, b) in t_en.extra.options.iter().zip(&t_es.extra.options) {
                        assert!(a.text != b.text && a.explain != b.explain, "{at}: option");
                        assert_eq!((a.correct, &a.arrows), (b.correct, &b.arrows), "{at}: overlays never change answers");
                    }
                    assert_eq!(t_en.solution, t_es.solution);
                }
            }
        }
        for k in ["guess", "count", "hanging", "choice", "square"] {
            assert!(kinds.contains(k), "no {k} task in the interactive courses");
        }
    }

    #[test]
    fn moves_tasks_serialize_without_extra_fields() {
        let mut t = task("moves");
        t.solution = vec!["e2e4".into()];
        let v = serde_json::to_value(&t).unwrap();
        let obj = v.as_object().unwrap();
        for k in ["game", "notes", "answer", "choices", "squares", "blind", "options"] {
            assert!(!obj.contains_key(k), "{k} should be omitted");
        }
        let back: Task = serde_json::from_value(serde_json::json!({
            "kind": "count", "prompt": "p", "answer": -3, "choices": [-3, 0, 3]
        }))
        .unwrap();
        assert_eq!(back.extra.answer, Some(-3));
        assert_eq!(back.extra.choices, vec![-3, 0, 3]);
    }
}
