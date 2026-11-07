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
    let de = c.localized(Lang::De);
    assert_eq!(de.lookup_opening(gm_engine::START_FEN).expect("start").opening.name, "Ausgangsstellung");
}

/// New languages with a partial overlay (fr: one opening, some fields) or no folder at all
/// (pt, de) still produce full views: every missing entry or field falls back to English,
/// while the start position is named in the view's language.
#[test]
fn partial_or_missing_overlays_fall_back_per_entry() {
    let base = Content::load(&fixture()).expect("load");
    let fr = base.localized(Lang::Fr);
    assert_eq!(fr.lang, Lang::Fr);
    let o = fr.opening("italian-game").expect("opening");
    assert_eq!(o.name, "Partie italienne");
    assert_eq!(o.ideas, vec!["Roquer tôt"]);
    assert_eq!(o.description, "Aim the bishop at f7.", "missing field falls back");
    assert_eq!(o.traps, vec!["Fried Liver"]);
    assert_eq!(o.uci, base.opening("italian-game").expect("en").uci);
    assert_eq!(fr.opening("giuoco-piano").expect("o").name, "Giuoco Piano", "missing entry falls back");
    assert_eq!(fr.course("basics").expect("c").title, "Chess Basics", "no courses overlay");
    assert_eq!(fr.endgame("kq-vs-k").expect("e").title, base.endgame("kq-vs-k").expect("e").title);
    assert_eq!(fr.lookup_opening(gm_engine::START_FEN).expect("start").opening.name, "Position initiale");

    for (lang, start) in [(Lang::Pt, "Posição inicial"), (Lang::De, "Ausgangsstellung")] {
        let v = base.localized(lang);
        assert_eq!(v.lang, lang);
        assert_eq!(v.course("basics").expect("c").title, "Chess Basics");
        assert_eq!(v.opening("italian-game").expect("o").name, "Italian Game");
        assert_eq!(v.courses.len(), base.courses.len());
        assert_eq!(v.openings.len(), base.openings.len());
        assert_eq!(v.endgames.len(), base.endgames.len());
        assert_eq!(v.lookup_opening(gm_engine::START_FEN).expect("start").opening.name, start);
        assert!(Arc::ptr_eq(&v, &fr.localized(lang)), "views reach their siblings");
    }
    let (ov, bad) = load_overlay(&fixture().join("i18n"), Lang::De);
    assert!(ov.is_empty() && bad == 0, "a missing folder is an empty overlay");
}

/// Every language gets a complete view of the shipped data, whatever overlays exist for it.
#[test]
fn shipped_data_localizes_in_every_language() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let base = Content::load_with_i18n(&data, &data.join("i18n")).expect("load");
    for lang in Lang::ALL {
        let v = base.localized(lang);
        assert_eq!(v.lang, lang);
        assert_eq!(v.courses.len(), base.courses.len(), "{lang}");
        assert_eq!(v.openings.len(), base.openings.len(), "{lang}");
        assert_eq!(v.endgames.len(), base.endgames.len(), "{lang}");
        for (a, b) in base.openings.iter().zip(&v.openings) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.uci, b.uci, "{lang}: overlays never change moves");
            assert!(!b.name.trim().is_empty(), "{lang}: {} has an empty name", a.id);
        }
    }
}

/// Records a gap when English has text but the overlay field is missing or blank.
fn need(gaps: &mut Vec<String>, en: &str, tr: Option<&String>, what: impl FnOnce() -> String) {
    if !en.trim().is_empty() && tr.is_none_or(|s| s.trim().is_empty()) {
        gaps.push(what());
    }
}

/// Same for lists (opening ideas/traps, endgame technique/lesson): same length, no blank items.
fn need_list(gaps: &mut Vec<String>, en: &[String], tr: Option<&Vec<String>>, what: impl FnOnce() -> String) {
    if en.is_empty() {
        return;
    }
    match tr {
        Some(v) if v.len() == en.len() && v.iter().all(|s| !s.trim().is_empty()) => {}
        Some(v) => gaps.push(format!("{} ({} of {} items)", what(), v.len(), en.len())),
        None => gaps.push(what()),
    }
}

/// Every shipped overlay (courses, openings, endgames, classics) translates every entry, every
/// lesson step, task text, list item, annotation and question, for every language besides
/// English. Puzzles have no content overlay: their themes are UI strings in `web/locales/`.
#[test]
fn shipped_overlays_are_complete_in_every_language() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let i18n = data.join("i18n");
    for lang in Lang::ALL.into_iter().filter(|l| *l != Lang::En) {
        let base = Content::load(&data).expect("load");
        let (ov, bad) = load_overlay(&i18n, lang);
        assert_eq!(bad, 0, "{lang}: unparsable overlay entries");
        let mut applied = base.clone();
        let st = apply_overlay(&mut applied, &ov);
        assert_eq!(st.ignored, 0, "{lang}: overlay entries for unknown ids or surplus steps: {st:?}");

        let mut gaps = Vec::new();
        for c in &base.courses {
            let Some(co) = ov.courses.get(&c.id) else {
                gaps.push(format!("course {}", c.id));
                continue;
            };
            need(&mut gaps, &c.title, co.title.as_ref(), || format!("course {}: title", c.id));
            need(&mut gaps, &c.description, co.description.as_ref(), || format!("course {}: description", c.id));
            for l in &c.lessons {
                let Some(lo) = co.lessons.get(&l.id) else {
                    gaps.push(format!("lesson {}/{}", c.id, l.id));
                    continue;
                };
                need(&mut gaps, &l.title, lo.title.as_ref(), || format!("lesson {}/{}: title", c.id, l.id));
                need(&mut gaps, &l.summary, lo.summary.as_ref(), || format!("lesson {}/{}: summary", c.id, l.id));
                if lo.steps.len() != l.steps.len() {
                    gaps.push(format!("lesson {}/{}: {} of {} steps", c.id, l.id, lo.steps.len(), l.steps.len()));
                }
                for (i, (s, so)) in l.steps.iter().zip(&lo.steps).enumerate() {
                    let at = || format!("lesson {}/{} step {}", c.id, l.id, i + 1);
                    let Some(so) = so else {
                        gaps.push(at());
                        continue;
                    };
                    need(&mut gaps, &s.text, so.text.as_ref(), || format!("{}: text", at()));
                    let Some(task) = &s.task else { continue };
                    let Some(to) = &so.task else {
                        gaps.push(format!("{}: task", at()));
                        continue;
                    };
                    need(&mut gaps, &task.prompt, to.prompt.as_ref(), || format!("{}: task prompt", at()));
                    need(&mut gaps, &task.success, to.success.as_ref(), || format!("{}: task success", at()));
                    if let Some(h) = &task.hint {
                        need(&mut gaps, h, to.hint.as_ref(), || format!("{}: task hint", at()));
                    }
                    if let Some(g) = &task.extra.game {
                        need(&mut gaps, g, to.game.as_ref(), || format!("{}: guess game", at()));
                    }
                    for (n, note) in task.extra.notes.iter().enumerate() {
                        need(&mut gaps, note, to.notes.get(n).and_then(Option::as_ref), || format!("{}: guess note {}", at(), n + 1));
                    }
                    for (n, opt) in task.extra.options.iter().enumerate() {
                        let oo = to.options.get(n).and_then(Option::as_ref);
                        need(&mut gaps, &opt.text, oo.and_then(|o| o.text.as_ref()), || format!("{}: option {} text", at(), n + 1));
                        need(&mut gaps, &opt.explain, oo.and_then(|o| o.explain.as_ref()), || format!("{}: option {} explanation", at(), n + 1));
                    }
                }
            }
        }
        for o in &base.openings {
            let Some(oo) = ov.openings.get(&o.id) else {
                gaps.push(format!("opening {}", o.id));
                continue;
            };
            need(&mut gaps, &o.name, oo.name.as_ref(), || format!("opening {}: name", o.id));
            need(&mut gaps, &o.description, oo.description.as_ref(), || format!("opening {}: description", o.id));
            need_list(&mut gaps, &o.ideas, oo.ideas.as_ref(), || format!("opening {}: ideas", o.id));
            need_list(&mut gaps, &o.traps, oo.traps.as_ref(), || format!("opening {}: traps", o.id));
        }
        for d in &base.endgames {
            let Some(eo) = ov.endgames.get(&d.id) else {
                gaps.push(format!("endgame {}", d.id));
                continue;
            };
            need(&mut gaps, &d.title, eo.title.as_ref(), || format!("endgame {}: title", d.id));
            need(&mut gaps, &d.description, eo.description.as_ref(), || format!("endgame {}: description", d.id));
            need(&mut gaps, &d.hint, eo.hint.as_ref(), || format!("endgame {}: hint", d.id));
            need(&mut gaps, &d.pitfall, eo.pitfall.as_ref(), || format!("endgame {}: pitfall", d.id));
            need_list(&mut gaps, &d.technique, eo.technique.as_ref(), || format!("endgame {}: technique", d.id));
            let steps: Vec<String> = d.lesson.iter().map(|s| s.text.clone()).collect();
            need_list(&mut gaps, &steps, eo.lesson.as_ref(), || format!("endgame {}: lesson steps", d.id));
        }
        for g in &base.classics {
            let Some(co) = ov.classics.get(&g.id) else {
                gaps.push(format!("classic {}", g.id));
                continue;
            };
            need(&mut gaps, &g.title, co.title.as_ref(), || format!("classic {}: title", g.id));
            need(&mut gaps, &g.event, co.event.as_ref(), || format!("classic {}: event", g.id));
            need(&mut gaps, &g.opening, co.opening.as_ref(), || format!("classic {}: opening", g.id));
            need(&mut gaps, &g.summary, co.summary.as_ref(), || format!("classic {}: summary", g.id));
            if co.annotations.len() != g.annotations.len() || co.questions.len() != g.questions.len() {
                gaps.push(format!("classic {}: annotation/question count", g.id));
            }
            for (i, (n, no)) in g.annotations.iter().zip(&co.annotations).enumerate() {
                need(&mut gaps, &n.text, no.as_ref().and_then(|x| x.text.as_ref()), || format!("classic {} note {}: text", g.id, i + 1));
                if let Some(label) = &n.label {
                    need(&mut gaps, label, no.as_ref().and_then(|x| x.label.as_ref()), || format!("classic {} note {}: label", g.id, i + 1));
                }
            }
            for (i, (q, qo)) in g.questions.iter().zip(&co.questions).enumerate() {
                let qo = qo.as_ref();
                need(&mut gaps, &q.prompt, qo.and_then(|x| x.prompt.as_ref()), || format!("classic {} question {}: prompt", g.id, i + 1));
                need(&mut gaps, &q.explanation, qo.and_then(|x| x.explanation.as_ref()), || format!("classic {} question {}: explanation", g.id, i + 1));
                if let Some(h) = &q.hint {
                    need(&mut gaps, h, qo.and_then(|x| x.hint.as_ref()), || format!("classic {} question {}: hint", g.id, i + 1));
                }
            }
        }
        assert!(gaps.is_empty(), "{lang}: {} untranslated item(s), e.g.\n  {}", gaps.len(), gaps.iter().take(25).cloned().collect::<Vec<_>>().join("\n  "));
    }
}

/// The README's content numbers match the shipped data, so the docs cannot drift silently.
#[test]
fn readme_numbers_match_shipped_data() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let readme = std::fs::read_to_string(root.join("README.md")).expect("README.md");
    let c = Content::load(&root.join("data")).expect("load");
    let lessons: usize = c.courses.iter().map(|x| x.lessons.len()).sum();
    let thousands = |n: usize| if n >= 1000 { format!("{},{:03}", n / 1000, n % 1000) } else { n.to_string() };
    let expected = [
        format!("**{} courses · {} interactive lessons.**", c.courses.len(), lessons),
        format!("**{} hand-picked tactics**", thousands(c.puzzles.len())),
        format!("Puzzles-{}-", thousands(c.puzzles.len()).replace(',', "%2C")),
        format!("**{} endgame drills**", c.endgames.len()),
        format!("{} of the most famous games", c.classics.len()),
        format!("**~{} named openings**", (c.openings.len() + 5) / 10 * 10),
    ];
    for e in expected {
        assert!(readme.contains(&e), "README.md should contain \"{e}\"");
    }
}
