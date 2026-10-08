//! Engine behaviour tests: mates, tactics, draws, limits, MultiPV.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gm_engine::shakmaty::{Chess, Position};
use gm_engine::{parse_fen, uci_to_move, Engine, Score, SearchInfo, SearchLimits};

fn search(e: &mut Engine, fen: &str, limits: SearchLimits) -> SearchInfo {
    let pos = parse_fen(fen).expect("fen");
    let stop = AtomicBool::new(false);
    e.search(&pos, &limits, &stop, &mut |_| {})
}

fn depth(d: u8) -> SearchLimits {
    SearchLimits {
        depth: Some(d),
        ..Default::default()
    }
}

fn movetime(ms: u64) -> SearchLimits {
    SearchLimits {
        movetime_ms: Some(ms),
        ..Default::default()
    }
}

#[test]
fn mate_in_one_white_and_black() {
    let mut e = Engine::new(8);
    let info = search(
        &mut e,
        "r1bqkbnr/pppp1ppp/2n5/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
        depth(4),
    );
    assert_eq!(info.best_move(), Some("h5f7"));
    assert_eq!(info.score(), Score::Mate(1));
    assert_eq!(info.best().unwrap().san[0], "Qxf7#");

    // Black to move: fool's mate. Score is white-POV, so negative.
    let info = search(
        &mut e,
        "rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq - 0 2",
        depth(4),
    );
    assert_eq!(info.best_move(), Some("d8h4"));
    assert_eq!(info.score(), Score::Mate(-1));
}

#[test]
fn mate_in_two() {
    let mut e = Engine::new(8);
    // Nf6+ gxf6 Bxf7#
    let info = search(
        &mut e,
        "r2qkb1r/pp2nppp/3p4/2pNN1B1/2BnP3/3P4/PPP2PPP/R2bK2R w KQkq - 1 1",
        depth(6),
    );
    assert_eq!(info.best_move(), Some("d5f6"));
    assert_eq!(info.score(), Score::Mate(2));
    // back-rank mate in 2 with a rook sacrifice
    let info = search(&mut e, "6k1/5ppp/8/8/8/8/1Q3PPP/3R2K1 w - - 0 1", depth(6));
    assert!(
        matches!(info.score(), Score::Mate(n) if (1..=2).contains(&n)),
        "{:?}",
        info.score()
    );
}

#[test]
fn mated_position_returns_no_lines() {
    let mut e = Engine::new(1);
    let info = search(
        &mut e,
        "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3",
        depth(5),
    );
    assert!(info.lines.is_empty());
    assert_eq!(info.best_move(), None);
}

#[test]
fn avoids_stalemate_and_mates_kqk() {
    let mut e = Engine::new(16);
    // Qb6 would be stalemate-ish territory: many queen moves stalemate the a8 king.
    let fen = "k7/8/8/1Q6/8/8/8/6K1 w - - 0 1";
    let info = search(&mut e, fen, movetime(300));
    let pos = parse_fen(fen).unwrap();
    let m = uci_to_move(&pos, info.best_move().unwrap()).unwrap();
    let mut after = pos.clone();
    after.play_unchecked(&m);
    assert!(
        !after.is_stalemate(),
        "engine stalemated with {:?}",
        info.best_move()
    );
    assert!(
        matches!(info.score(), Score::Mate(n) if n > 0),
        "{:?}",
        info.score()
    );

    // Play KQK out from the centre: the engine must deliver mate (never stalemate).
    let mut pos: Chess = parse_fen("8/8/8/4k3/8/8/8/4K2Q w - - 0 1").unwrap();
    let stop = AtomicBool::new(false);
    let mut mated = false;
    for _ply in 0..80 {
        if pos.is_game_over() {
            mated = pos.is_checkmate();
            break;
        }
        let info = e.search(&pos, &movetime(150), &stop, &mut |_| {});
        let m = uci_to_move(&pos, info.best_move().unwrap()).unwrap();
        pos.play_unchecked(&m);
    }
    if pos.is_game_over() {
        mated = pos.is_checkmate();
    }
    assert!(mated, "KQK not converted: {}", gm_engine::to_fen(&pos));
}

#[test]
fn krk_is_won() {
    let mut e = Engine::new(16);
    let info = search(&mut e, "8/8/8/3k4/8/8/8/R3K3 w - - 0 1", movetime(400));
    match info.score() {
        Score::Cp(c) => assert!(c > 300, "{c}"),
        Score::Mate(n) => assert!(n > 0),
    }
}

#[test]
fn tactics_wac() {
    // (fen, accepted best moves in UCI)
    let cases: &[(&str, &[&str])] = &[
        (
            "2rr3k/pp3pp1/1nnqbN1p/3pN3/2pP4/2P3Q1/PPB4P/R4RK1 w - - 0 1",
            &["g3g6"],
        ),
        ("8/7p/5k2/5p2/p1p2P2/Pr1pPK2/1P1R3P/8 b - - 0 1", &["b3b2"]),
        (
            "5rk1/1ppb3p/p1pb4/6q1/3P1p1r/2P1R2P/PP1BQ1P1/5RKN w - - 0 1",
            &["e3g3"],
        ),
        (
            "r1bq2rk/pp3pbp/2p1p1pQ/7P/3P4/2PB1N2/PP3PPR/2KR4 w - - 0 1",
            &["h6h7"],
        ),
        (
            "5k2/6pp/p1qN4/1p1p4/3P4/2PKP2Q/PP3r2/3R4 b - - 0 1",
            &["c6c4"],
        ),
        ("7k/p7/1R5K/6r1/6p1/6P1/8/8 w - - 0 1", &["b6b7"]),
        (
            "rnbqkb1r/pppp1ppp/8/4P3/6n1/7P/PPPNPPP1/R1BQKBNR b KQkq - 0 1",
            &["g4e3"],
        ),
        (
            "r4q1k/p2bR1rp/2p2Q1N/5p2/5p2/2P5/PP3PPP/R5K1 w - - 0 1",
            &["e7f7"],
        ),
        (
            "3q1rk1/p4pp1/2pb3p/3p4/6Pr/1PNQ4/P1PB1PP1/4RRK1 b - - 0 1",
            &["d6h2"],
        ),
        (
            "2br2k1/2q3rn/p2NppQ1/2p1P3/Pp5R/4P3/1P3PPP/3R2K1 w - - 0 1",
            &["h4h7"],
        ),
    ];
    let mut e = Engine::new(32);
    let mut solved = 0;
    let mut failures = Vec::new();
    for (fen, bm) in cases {
        e.new_game();
        let info = search(&mut e, fen, movetime(1500));
        let best = info.best_move().unwrap_or("-").to_string();
        if bm.contains(&best.as_str()) {
            solved += 1;
        } else {
            failures.push(format!("{fen}: got {best} ({}) want {bm:?}", info.score()));
        }
    }
    eprintln!(
        "WAC solved {solved}/{}; failures: {failures:#?}",
        cases.len()
    );
    assert!(solved >= 9, "solved only {solved}: {failures:#?}");
}

#[test]
fn respects_movetime() {
    let mut e = Engine::new(16);
    let t = Instant::now();
    let info = search(
        &mut e,
        "r1bq1rk1/pp2bppp/2n1pn2/3p4/2PP4/2N1PN2/PP1B1PPP/R2QKB1R w KQ - 0 8",
        movetime(300),
    );
    let el = t.elapsed();
    assert!(el < Duration::from_millis(450), "took {el:?}");
    assert!(info.depth >= 6, "depth {}", info.depth);
    assert!(info.best_move().is_some());
}

#[test]
fn respects_node_limit() {
    let mut e = Engine::new(4);
    let info = search(
        &mut e,
        gm_engine::START_FEN,
        SearchLimits {
            nodes: Some(3000),
            ..Default::default()
        },
    );
    assert!(info.best_move().is_some());
    assert!(info.nodes <= 3000 + 200, "nodes {}", info.nodes);
    // A tiny limit still yields a move (depth 1 always completes).
    let info = search(
        &mut e,
        gm_engine::START_FEN,
        SearchLimits {
            nodes: Some(1),
            ..Default::default()
        },
    );
    assert!(info.best_move().is_some());
}

#[test]
fn stop_flag_interrupts_infinite_search() {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&stop);
    let h = std::thread::spawn(move || {
        let mut e = Engine::new(8);
        let pos = Chess::default();
        let mut infos = 0;
        let info = e.search(&pos, &SearchLimits::default(), &s2, &mut |_| infos += 1);
        (info, infos)
    });
    std::thread::sleep(Duration::from_millis(200));
    let t = Instant::now();
    stop.store(true, Ordering::Relaxed);
    let (info, infos) = h.join().unwrap();
    assert!(t.elapsed() < Duration::from_millis(100));
    assert!(infos >= 1);
    assert!(info.best_move().is_some());

    // stop already set: still returns a depth-1 result
    let mut e = Engine::new(1);
    let stop = AtomicBool::new(true);
    let info = e.search(
        &Chess::default(),
        &SearchLimits::default(),
        &stop,
        &mut |_| {},
    );
    assert_eq!(info.depth, 1);
    assert!(info.best_move().is_some());
}

#[test]
fn multipv_lines_distinct_and_sorted() {
    let mut e = Engine::new(16);
    let mut calls = 0;
    let pos = Chess::default();
    let stop = AtomicBool::new(false);
    let info = e.search(
        &pos,
        &SearchLimits {
            depth: Some(8),
            multipv: 4,
            ..Default::default()
        },
        &stop,
        &mut |i| {
            calls += 1;
            assert_eq!(i.lines.len(), 4);
        },
    );
    assert_eq!(calls, 8);
    assert_eq!(info.depth, 8);
    assert_eq!(info.lines.len(), 4);
    let firsts: Vec<_> = info.lines.iter().map(|l| l.moves[0].clone()).collect();
    let mut dedup = firsts.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(dedup.len(), 4, "{firsts:?}");
    for w in info.lines.windows(2) {
        assert!(w[0].score.to_cp() >= w[1].score.to_cp());
    }
    for l in &info.lines {
        assert_eq!(l.moves.len(), l.san.len());
        assert!(!l.moves.is_empty());
    }
    // multipv larger than the number of legal moves
    let info = search(
        &mut e,
        "7k/8/8/8/8/8/8/K7 w - - 0 1",
        SearchLimits {
            depth: Some(3),
            multipv: 10,
            ..Default::default()
        },
    );
    assert_eq!(info.lines.len(), 3);
}

#[test]
fn draws_are_scored_zero() {
    let mut e = Engine::new(4);
    // K+B vs K: insufficient material
    let info = search(&mut e, "8/8/4k3/8/8/3BK3/8/8 w - - 0 1", depth(6));
    assert_eq!(info.score(), Score::Cp(0));
    // 50-move rule: every white move reaches halfmove 100 (no mate in one available).
    let info = search(&mut e, "8/8/8/4k3/8/8/8/4K2Q w - - 99 80", depth(6));
    assert_eq!(info.score(), Score::Cp(0), "{:?}", info.best());
    // ...but a fresh clock is clearly winning
    let info = search(&mut e, "8/8/8/4k3/8/8/8/4K2Q w - - 0 80", depth(6));
    assert!(info.score().to_cp() > 500, "{:?}", info.score());
}

#[test]
fn repetition_with_game_history() {
    let mut e = Engine::new(4);
    let stop = AtomicBool::new(false);
    // White is down material but can repeat; history makes the repeat immediate.
    let start = Chess::default();
    let moves = ["g1f3", "g8f6", "f3g1", "f6g8"];
    let mut hist = vec![];
    let mut pos = start.clone();
    for u in moves {
        hist.push(pos.clone());
        let m = uci_to_move(&pos, u).unwrap();
        pos.play_unchecked(&m);
    }
    let info = e.search_with_history(&pos, &hist, &depth(6), &stop, &mut |_| {});
    assert!(info.best_move().is_some());
}

#[test]
fn depth_in_one_second_from_startpos() {
    let mut e = Engine::new(64);
    let info = search(&mut e, gm_engine::START_FEN, movetime(1000));
    eprintln!(
        "startpos 1s: depth {} seldepth {} nodes {} nps {} pv {}",
        info.depth,
        info.seldepth,
        info.nodes,
        info.nps,
        info.best().map(|l| l.san.join(" ")).unwrap_or_default()
    );
    assert!(info.depth >= 12, "depth {}", info.depth);
}

#[test]
fn bad_input_never_panics() {
    for f in [
        "",
        "x",
        "8/8/8/8/8/8/8/8 w - - 0 1",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0",
        "k7/8/8/8/8/8/8/K7 w - - 0 1 extra",
    ] {
        let _ = parse_fen(f);
    }
    let pos = Chess::default();
    for u in ["", "e2", "e2e9", "a1a1", "e7e8q", "e2e4qq", "😀😀😀😀"] {
        assert!(uci_to_move(&pos, u).is_err(), "{u}");
    }
}
