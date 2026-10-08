//! Translation overlays: merge across files, fallback to English, never touch moves/FENs.

use std::path::PathBuf;
use std::sync::Arc;

use gm_content::overlay::{apply_overlay, load_overlay};
use gm_content::{Content, Lang};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/data")
}

#[test]
fn overlays_merge_and_fall_back() {
    let base = Content::load(&fixture()).expect("load");
    assert_eq!(base.lang, Lang::En);
    let es = base.localized(Lang::Es);
    let en = base.localized(Lang::En);
    assert_eq!(es.lang, Lang::Es);

    // Courses: fields from both files are merged; missing fields fall back to English.
    let c = es.course("basics").expect("course");
    assert_eq!(c.title, "Fundamentos del ajedrez");
    assert_eq!(c.description, "Aprende las reglas.");
    assert_eq!(c.icon, "♟️");
    let l = &c.lessons[0];
    assert_eq!(l.title, "El peón"); // from part1, deep-merged with part2
    assert_eq!(l.summary, "Cómo se mueven los peones.");
    assert_eq!(l.steps.len(), 3, "overlays never add or drop steps");
    assert_eq!(l.steps[0].text, "Los peones avanzan hacia delante.");
    assert_eq!(l.steps[1].text, "Avanza el peón dos casillas.");
    assert_eq!(l.steps[2].text, "Untranslated step.");
    // Never take FENs, arrows or solutions from an overlay.
    let en_course = base.course("basics").expect("course");
    assert_eq!(l.steps[0].fen, en_course.lessons[0].steps[0].fen);
    assert_eq!(l.steps[0].arrows, en_course.lessons[0].steps[0].arrows);
    let task = l.steps[1].task.as_ref().expect("task");
    assert_eq!(task.prompt, "Juega e4");
    assert_eq!(task.hint.as_deref(), Some("¡Dos casillas!"));
    assert_eq!(task.success, "¡Genial!");
    assert_eq!(task.solution, vec!["e2e4".to_string()]);

    // Openings: names, ideas; moves untouched; unknown ids and malformed entries ignored.
    let o = es.opening("italian-game").expect("opening");
    assert_eq!(o.name, "Apertura italiana");
    assert_eq!(o.ideas, vec!["Enroca pronto", "Juega c3 y d4"]);
    assert_eq!(o.traps, vec!["Fried Liver"], "missing list falls back");
    assert_eq!(o.description, "Aim the bishop at f7.");
    assert_eq!(o.moves, "e4 e5 Nf3 Nc6 Bc4");
    assert_eq!(o.uci, base.opening("italian-game").expect("en").uci);
    assert_eq!(es.opening("sicilian").expect("sicilian").name, "Sicilian Defense");
    assert!(es.opening("not-an-opening").is_none());

    // Endgames: blank strings fall back.
    let d = es.endgame("kq-vs-k").expect("endgame");
    assert_eq!(d.title, "Mate de dama");
    assert_eq!(d.hint, "Box the king.");
    assert_eq!(d.technique, vec!["Restringe al rey"]);
    assert_eq!(d.fen, base.endgame("kq-vs-k").expect("en").fen);

    // English view is untouched, and the source content too.
    assert_eq!(en.course("basics").expect("c").title, "Chess Basics");
    assert_eq!(base.opening("italian-game").expect("o").name, "Italian Game");

    // Opening book lookups report localized names (current + continuations + start position).
    let fen = base.opening("italian-game").expect("o").fen.clone();
    let m = es.lookup_opening(&fen).expect("in book");
    assert_eq!(m.opening.name, "Apertura italiana");
    let bc5 = m.continuations.iter().find(|b| b.san == "Bc5").expect("Bc5");
    assert_eq!(bc5.name.as_deref(), Some("Giuoco piano"));
    assert_eq!(es.lookup_opening(gm_engine::START_FEN).expect("start").opening.name, "Posición inicial");
    assert_eq!(base.lookup_opening(&fen).expect("en").opening.name, "Italian Game");
    assert_eq!(es.opening_ref("starting-position").expect("start").name, "Posición inicial");

    // Views are shared, and a view can reach its siblings.
    assert!(Arc::ptr_eq(&es, &base.localized(Lang::Es)));
    assert!(Arc::ptr_eq(&en, &es.localized(Lang::En)));
}

#[test]
fn overlay_stats_count_applied_and_ignored() {
    let mut c = Content::load(&fixture()).expect("load");
    let (ov, bad) = load_overlay(&fixture().join("i18n"), Lang::Es);
    assert_eq!(bad, 1, "the malformed sicilian entry");
    let st = apply_overlay(&mut c, &ov);
    // applied: course, lesson, step 0, step 1, italian, giuoco, endgame
    assert_eq!(st.applied, 7, "{st:?}");
    // ignored: unknown lesson, surplus step, unknown course, unknown opening
    assert_eq!(st.ignored, 4, "{st:?}");
}

#[test]
fn missing_language_folder_is_english() {
    let base = Content::load(&fixture()).expect("load");
    let (ov, bad) = load_overlay(&fixture().join("i18n"), Lang::En);
    assert!(ov.is_empty() && bad == 0);
    let en = base.localized(Lang::En);
    assert_eq!(en.course("basics").expect("c").title, "Chess Basics");
}

/// The shipped Spanish endgame overlay translates every drill: text, pitfall and each lesson step.
#[test]
fn spanish_endgames_are_complete() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let base = Content::load_with_i18n(&data, &data.join("i18n")).expect("load");
    let es = base.localized(Lang::Es);
    assert_eq!(es.endgames.len(), base.endgames.len());
    for (en, es) in base.endgames.iter().zip(&es.endgames) {
        assert_eq!(en.id, es.id);
        assert_ne!(en.title, es.title, "{}: title", en.id);
        assert_ne!(en.pitfall, es.pitfall, "{}: pitfall", en.id);
        assert_eq!(en.lesson.len(), es.lesson.len());
        for (i, (a, b)) in en.lesson.iter().zip(&es.lesson).enumerate() {
            assert_ne!(a.text, b.text, "{}: lesson step {} untranslated", en.id, i + 1);
            assert_eq!(a.fen, b.fen);
            assert_eq!(a.arrows, b.arrows);
        }
        assert_eq!(en.variants, es.variants);
        assert_eq!(en.success, es.success);
    }
}

#[test]
fn hand_built_content_localizes_without_overlays() {
    let c = Content::default();
    let es = c.localized(Lang::Es);
    assert_eq!(es.lookup_opening(gm_engine::START_FEN).expect("start").opening.name, "Posición inicial");
    assert_eq!(c.lookup_opening(gm_engine::START_FEN).expect("start").opening.name, "Starting Position");
}
