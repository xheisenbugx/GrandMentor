use super::*;
use std::collections::HashSet;
use std::path::PathBuf;

use gm_content::Content;
use gm_engine::{to_fen, START_FEN};

fn content() -> Content {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    Content::load(&dir).unwrap_or_default()
}

/// Random but reproducible game positions (start FEN + move list), none game-over.
fn random_positions(n: usize, seed: u64) -> Vec<(String, Vec<String>)> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut out = Vec::new();
    while out.len() < n {
        let target = rng.gen_range(0..60);
        let mut pos = Chess::default();
        let mut moves = Vec::new();
        for _ in 0..target {
            let legal = pos.legal_moves();
            if legal.is_empty() {
                break;
            }
            let m = legal[rng.gen_range(0..legal.len())].clone();
            moves.push(move_to_uci(&m));
            pos.play_unchecked(&m);
        }
        if pos.is_game_over() {
            continue;
        }
        // Mix: half as move lists from the start, half as a bare FEN.
        if out.len() % 2 == 0 {
            out.push((START_FEN.to_string(), moves));
        } else {
            out.push((to_fen(&pos), Vec::new()));
        }
    }
    out
}

#[test]
fn roster_is_valid() {
    let bots = list(Lang::En);
    assert!(bots.len() >= 14, "expected ~14+ bots, got {}", bots.len());
    let ids: HashSet<_> = bots.iter().map(|b| b.id.clone()).collect();
    assert_eq!(ids.len(), bots.len(), "duplicate bot ids");
    assert!(bots.iter().any(|b| b.elo <= 300));
    assert!(bots.iter().any(|b| b.elo >= 3000));
    assert!(bots.iter().any(|b| b.category == "coach"));
    for b in &bots {
        assert!(!b.name.is_empty() && !b.avatar.is_empty() && !b.greeting.is_empty() && !b.description.is_empty());
        assert!(["beginner", "intermediate", "advanced", "master", "coach"].contains(&b.category.as_str()));
        assert!(["beginner", "aggressive", "positional", "defensive", "trappy", "universal", "coach"].contains(&b.style.as_str()));
        assert_eq!(get(&b.id, Lang::En).as_ref(), Some(b));
    }
    for p in PERSONAS {
        for lang in Lang::ALL {
            let l = p.lines(lang);
            assert!(!l.description.is_empty() && !l.greeting.is_empty(), "{} {lang}: missing text", p.id);
            for lines in [l.capture, l.captured, l.blunder, l.check, l.winning, l.losing, l.win, l.opening] {
                assert!(!lines.is_empty(), "{} {lang}: missing chat lines", p.id);
            }
            for line in l.opening {
                assert!(line.contains("{opening}"), "{} {lang}: opening line without placeholder", p.id);
            }
        }
    }
    // JSON shape
    let v = serde_json::to_value(&bots[0]).expect("serialize");
    for k in ["id", "name", "elo", "avatar", "style", "description", "greeting", "category"] {
        assert!(v.get(k).is_some(), "missing {k}");
    }
}

#[test]
fn every_bot_plays_legal_moves() {
    let content = content();
    let mut engine = Engine::new(8);
    let positions = random_positions(20, 7);
    let mut rng = StdRng::seed_from_u64(42);
    for b in list(Lang::En) {
        for (fen, moves) in &positions {
            let r = choose_move_with(&mut engine, &content, &b.id, fen, moves, &mut rng, Some(15), Lang::En)
                .unwrap_or_else(|e| panic!("{} failed on {fen} {moves:?}: {e}", b.id));
            let game = replay(fen, moves).expect("replay");
            let m = uci_to_move(&game.pos, &r.uci).unwrap_or_else(|e| panic!("{} illegal {}: {e}", b.id, r.uci));
            assert_eq!(r.san, move_to_san(&game.pos, &m));
            assert!((300..=2500).contains(&r.think_ms), "think_ms {}", r.think_ms);
        }
    }
}

#[test]
fn errors_are_graceful() {
    let content = content();
    let mut engine = Engine::new(1);
    // Fool's mate: game over.
    let mated: Vec<String> = ["f2f3", "e7e5", "g2g4", "d8h4"].iter().map(|s| s.to_string()).collect();
    assert!(choose_move(&mut engine, &content, "titan", START_FEN, &mated, Lang::En).is_err());
    // Stalemate FEN.
    assert!(choose_move(&mut engine, &content, "pawnny", "7k/5Q2/6K1/8/8/8/8/8 b - - 0 1", &[], Lang::En).is_err());
    assert!(choose_move(&mut engine, &content, "nobody", START_FEN, &[], Lang::En).is_err());
    assert!(choose_move(&mut engine, &content, "titan", "garbage", &[], Lang::En).is_err());
    assert!(choose_move(&mut engine, &content, "titan", START_FEN, &["e2e5".into()], Lang::En).is_err());
    assert!(choose_move(&mut engine, &content, "titan", START_FEN, &["zz".into()], Lang::En).is_err());
    // Moves after the game is over are rejected.
    let mut over = mated.clone();
    over.push("a2a3".into());
    assert!(choose_move(&mut engine, &content, "titan", START_FEN, &over, Lang::En).is_err());
}

#[test]
fn forced_move_is_played() {
    let content = content();
    let mut engine = Engine::new(1);
    // Black king in check with exactly one legal move.
    let fen = "k7/8/1K6/8/8/8/8/R7 b - - 0 1";
    let pos = parse_fen(fen).expect("fen");
    let legal = pos.legal_moves();
    if legal.len() == 1 {
        let r = choose_move(&mut engine, &content, "titan", fen, &[], Lang::En).expect("move");
        assert_eq!(r.uci, move_to_uci(&legal[0]));
    }
}

#[test]
fn strong_bot_takes_mate_in_one() {
    let content = content();
    let mut engine = Engine::new(4);
    // Scholar's mate available: Qxf7#.
    let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4";
    let mut rng = StdRng::seed_from_u64(1);
    for _ in 0..5 {
        let r = choose_move_with(&mut engine, &content, "titan", fen, &[], &mut rng, Some(200), Lang::En).expect("move");
        assert_eq!(r.uci, "h5f7");
        assert!(r.chat.is_some(), "mate should come with a win line");
    }
}

#[test]
fn book_is_used_from_start() {
    let content = content();
    if content.openings.is_empty() {
        return;
    }
    let first: HashSet<String> = content.openings.iter().filter_map(|o| o.uci.first().cloned()).collect();
    let mut engine = Engine::new(1);
    let mut rng = StdRng::seed_from_u64(3);
    for _ in 0..10 {
        let r = choose_move_with(&mut engine, &content, "titan", START_FEN, &[], &mut rng, Some(20), Lang::En).expect("move");
        assert!(first.contains(&r.uci), "titan should play a book move, got {}", r.uci);
    }
}

#[test]
fn coach_always_talks_and_spots_loose_pieces() {
    let content = content();
    let mut engine = Engine::new(4);
    let mut rng = StdRng::seed_from_u64(9);
    for (fen, moves) in random_positions(8, 11) {
        let r = choose_move_with(&mut engine, &content, "coach", &fen, &moves, &mut rng, Some(15), Lang::En).expect("move");
        assert!(r.chat.as_deref().map(|c| !c.is_empty()).unwrap_or(false), "coach must give a tip");
    }
    // White knight on e5 attacked by the d6 pawn and undefended.
    let pos = parse_fen("4k3/8/3p4/4N3/8/8/8/4K3 w - - 0 1").expect("fen");
    let loose = loose_pieces(&pos, Color::White);
    assert_eq!(loose, vec![(Role::Knight, Square::E5)]);
    // Defended by a pawn and attacked by a rook only: not loose.
    let pos = parse_fen("4k3/8/8/4r3/8/2N5/1P6/4K3 w - - 0 1").expect("fen");
    assert!(loose_pieces(&pos, Color::White).is_empty());
}

#[test]
fn weak_bots_are_more_random_than_strong() {
    // From a quiet middlegame, a strong bot's choices are concentrated; a weak bot's are spread.
    let content = Content::default();
    let mut engine = Engine::new(4);
    let fen = "r1bq1rk1/pp2bppp/2n1pn2/3p4/2PP4/2N2N2/PP2BPPP/R2QKB1R w KQ - 0 9";
    let distinct = |id: &str| {
        let mut rng = StdRng::seed_from_u64(5);
        let mut engine2 = Engine::new(4);
        let mut set = HashSet::new();
        for _ in 0..25 {
            let r = choose_move_with(&mut engine2, &content, id, fen, &[], &mut rng, Some(15), Lang::En).expect("move");
            set.insert(r.uci);
        }
        set.len()
    };
    let _ = &mut engine;
    assert!(distinct("pawnny") > distinct("titan"));
}

/// Play one game between two bots; returns +1 if `a` wins, -1 if `b` wins, 0 otherwise
/// (adjudicated by material after `max_plies`).
#[allow(clippy::too_many_arguments)]
fn play_game(engine: &mut Engine, content: &Content, a: &str, b: &str, a_white: bool, seed: u64, max_plies: usize, cap_ms: u64) -> i32 {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut moves: Vec<String> = Vec::new();
    let mut pos = Chess::default();
    for _ in 0..max_plies {
        if pos.is_game_over() {
            break;
        }
        let white_to_move = pos.turn() == Color::White;
        let id = if white_to_move == a_white { a } else { b };
        let r = choose_move_with(engine, content, id, START_FEN, &moves, &mut rng, Some(cap_ms), Lang::En).expect("bot move");
        let m = uci_to_move(&pos, &r.uci).expect("legal");
        pos.play_unchecked(&m);
        moves.push(r.uci);
    }
    let a_color = if a_white { Color::White } else { Color::Black };
    if pos.is_checkmate() {
        return if pos.turn() == a_color { -1 } else { 1 };
    }
    if pos.is_game_over() {
        return 0;
    }
    let material = |c: Color| -> i32 {
        pos.board().by_color(c).into_iter().filter_map(|s| pos.board().role_at(s)).map(value).sum()
    };
    let diff = material(a_color) - material(!a_color);
    match diff {
        d if d >= 300 => 1,
        d if d <= -300 => -1,
        _ => 0,
    }
}

#[test]
fn strong_bot_beats_weakest_quick() {
    let content = content();
    let mut engine = Engine::new(8);
    let mut score = 0;
    for g in 0..4u64 {
        score += play_game(&mut engine, &content, "titan", "pawnny", g % 2 == 0, g, 80, 10);
    }
    assert!(score >= 2, "titan should dominate pawnny, score {score}");
}

#[test]
#[ignore = "slow: full-length self-play match"]
fn strong_bot_beats_weakest_match() {
    let content = content();
    let mut engine = Engine::new(32);
    let mut score = 0;
    for g in 0..10u64 {
        score += play_game(&mut engine, &content, "athena", "lulu", g % 2 == 0, 100 + g, 200, 100);
    }
    assert!(score >= 6, "athena should beat lulu, score {score}");
}

#[test]
#[ignore = "slow: rating ladder sanity check"]
fn ladder_mid_vs_weak() {
    let content = content();
    let mut engine = Engine::new(32);
    let mut score = 0;
    for g in 0..10u64 {
        score += play_game(&mut engine, &content, "oliver", "pawnny", g % 2 == 0, 200 + g, 160, 60);
    }
    assert!(score >= 5, "oliver should beat pawnny, score {score}");
}

#[test]
fn spanish_roster_and_greetings() {
    let en = list(Lang::En);
    let es = list(Lang::Es);
    assert_eq!(en.len(), es.len());
    for (a, b) in en.iter().zip(&es) {
        // Identity never changes with the language; only the text does.
        assert_eq!((&a.id, &a.name, a.elo, &a.avatar, &a.style, &a.category), (&b.id, &b.name, b.elo, &b.avatar, &b.style, &b.category));
        assert_ne!(a.greeting, b.greeting, "{}: greeting not translated", a.id);
        assert_ne!(a.description, b.description, "{}: description not translated", a.id);
    }
    let pawnny = get("pawnny", Lang::Es).expect("pawnny");
    assert_eq!(pawnny.name, "Pawnny");
    assert!(pawnny.greeting.starts_with("¡Hola"), "{}", pawnny.greeting);
    let mia = get("coach", Lang::Es).expect("coach");
    assert!(mia.greeting.contains("soy Mia"), "{}", mia.greeting);
}

#[test]
fn spanish_coach_tips() {
    let content = content();
    let mut engine = Engine::new(4);
    let mut rng = StdRng::seed_from_u64(3);
    for (fen, moves) in random_positions(6, 21) {
        let r = choose_move_with(&mut engine, &content, "coach", &fen, &moves, &mut rng, Some(15), Lang::Es).expect("move");
        let chat = r.chat.unwrap_or_default();
        assert!(!chat.is_empty());
        for w in [" the ", "Your ", " your ", "Check!"] {
            assert!(!chat.contains(w), "English leaked into Spanish tip: {chat}");
        }
    }
    // The coach grabbing an undefended knight explains it with the right gender.
    let tip = tip_text(Lang::Es);
    let mut vars = PieceRef::new(Role::Rook).vars("p", Lang::Es);
    vars.extend(kv(&[("sq", "a1")]));
    assert_eq!(fill(tip.loose, &vars), "Atención: tu torre en a1 está atacada y mal defendida.");
    let mut vars = PieceRef::new(Role::Bishop).vars("p", Lang::Es);
    vars.extend(kv(&[("sq", "c4")]));
    assert!(fill(tip.took_loose, &vars).starts_with("Tu alfil en c4 no estaba protegido, así que me lo llevé."));
}

#[test]
fn spanish_opening_chat_uses_localized_names() {
    // Max (trappy) in the opening often names the opening; the name must come from the
    // Spanish view (which falls back to English when untranslated) and the line must be Spanish.
    let content = content();
    let es = content.localized(Lang::Es);
    let mut engine = Engine::new(4);
    let mut rng = StdRng::seed_from_u64(1);
    let mut seen = false;
    for _ in 0..40 {
        let r = choose_move_with(&mut engine, &content, "max", START_FEN, &["e2e4".into()], &mut rng, Some(10), Lang::Es).expect("move");
        if let Some(chat) = r.chat {
            if es.openings.iter().any(|o| chat.contains(&o.name)) {
                assert!(chat.contains("secretos") || chat.contains("coto de caza"), "{chat}");
                seen = true;
            }
        }
    }
    assert!(seen, "max never named an opening");
}

#[test]
fn adaptive_bot_plays_at_requested_level() {
    let content = Content::default();
    let mut engine = Engine::new(4);
    assert!(exists(ADAPTIVE_ID));
    let p = get(ADAPTIVE_ID, Lang::Es).expect("adaptive bot");
    assert_eq!(p.elo, ADAPTIVE_START_ELO);
    let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4";
    // At a strong level it finds the mate in one every time; out-of-range levels are clamped.
    let mut rng = StdRng::seed_from_u64(3);
    for elo in [2800u16, u16::MAX] {
        let r = choose_move_inner(&mut engine, &content, ADAPTIVE_ID, fen, &[], &mut rng, Some(200), Some(elo), Lang::En)
            .expect("move");
        assert_eq!(r.uci, "h5f7");
    }
    for elo in [0u16, 250, 1200] {
        let r = choose_move_at(&mut engine, &content, ADAPTIVE_ID, START_FEN, &[], Some(elo), Lang::En).expect("move");
        assert!(!r.uci.is_empty());
    }
}
