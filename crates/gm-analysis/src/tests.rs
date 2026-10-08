use super::*;

fn ucis(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

fn pool() -> EnginePool {
    EnginePool::new(2, 16)
}

const SCHOLAR: &str = "e2e4 e7e5 f1c4 b8c6 d1h5 g8f6 h5f7";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scholars_mate_review() {
    let content = Arc::new(gm_content::Content::default());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let review = review_game(&pool(), content, gm_engine::START_FEN, &ucis(SCHOLAR), 8, Some(tx), Lang::En)
        .await
        .expect("review");

    assert_eq!(review.moves.len(), 7);
    assert_eq!(review.evals.len(), 8);
    let nf6 = &review.moves[5];
    assert_eq!(nf6.san, "Nf6");
    assert_eq!(nf6.color, "black");
    assert_eq!(nf6.classification, Classification::Blunder, "{nf6:?}");
    assert!(nf6.win_chance_loss > 20.0);
    assert!(!nf6.best_move_san.is_empty());

    let qxf7 = &review.moves[6];
    assert_eq!(qxf7.san, "Qxf7#");
    assert!(
        matches!(qxf7.classification, Classification::Best | Classification::Great),
        "{:?}",
        qxf7.classification
    );
    assert_eq!(qxf7.win_chance_loss, 0.0);
    // Final position: black is mated -> white-favouring mate score.
    assert_eq!(review.evals[7], Score::Mate(0));
    assert_eq!(win_percent(review.evals[7]), 100.0);

    // Accuracy bounds.
    // (Absolute values depend on engine strength; with a real engine white is ~90+.)
    assert!(review.white.accuracy > 40.0, "white {}", review.white.accuracy);
    assert!(review.white.accuracy <= 100.0);
    assert!(review.black.accuracy < review.white.accuracy, "black {}", review.black.accuracy);
    assert!(review.black.accuracy >= 0.0);
    assert!(review.white.estimated_elo > review.black.estimated_elo);
    assert_eq!(review.black.counts.get("blunder"), Some(&1));
    assert_eq!(review.white.counts.values().sum::<u32>(), 4);
    assert_eq!(review.black.counts.values().sum::<u32>(), 3);

    assert!(review.key_moments.contains(&6), "{:?}", review.key_moments);
    assert!(!review.summary.is_empty());
    assert!(review.summary.contains("Nf6"), "{}", review.summary);
    for m in &review.moves {
        assert!(!m.explanation.is_empty());
    }

    // Progress: monotone, ends at 1.0.
    let mut last = 0.0f32;
    let mut values = Vec::new();
    while let Ok(p) = rx.try_recv() {
        assert!(p >= last && p <= 1.0);
        last = p;
        values.push(p);
    }
    assert_eq!(values.last().copied(), Some(1.0));

    // JSON shape.
    let json = serde_json::to_value(&review).expect("json");
    assert_eq!(json["moves"][5]["classification"], "blunder");
    assert!(json["white"]["counts"]["brilliant"].is_number());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn black_mates_is_negative() {
    // Fool's mate.
    let content = Arc::new(gm_content::Content::default());
    let review = review_game(&pool(), content, "start", &ucis("f2f3 e7e5 g2g4 d8h4"), 6, None, Lang::En)
        .await
        .expect("review");
    let last = review.evals.last().copied().expect("evals");
    assert!(win_percent(last) < 5.0, "{last:?}");
    assert_eq!(review.moves[2].classification, Classification::Blunder, "{:?}", review.moves[2]);
    assert!(review.summary.contains("Black"), "{}", review.summary);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_input_is_error_not_panic() {
    let content = Arc::new(gm_content::Content::default());
    let p = pool();
    assert!(review_game(&p, content.clone(), "garbage", &[], 4, None, Lang::En).await.is_err());
    assert!(review_game(&p, content.clone(), "start", &ucis("e2e5"), 4, None, Lang::En).await.is_err());
    assert!(review_game(&p, content.clone(), "start", &ucis("e2e4 zzzz"), 4, None, Lang::En).await.is_err());
    let too_long = vec!["e2e4".to_string(); MAX_PLIES + 1];
    assert!(review_game(&p, content.clone(), "start", &too_long, 4, None, Lang::En).await.is_err());
    // Empty game is fine.
    let r = review_game(&p, content, "start", &[], 4, None, Lang::En).await.expect("empty");
    assert!(r.moves.is_empty());
    assert_eq!(r.evals.len(), 1);
    assert_eq!(r.white.accuracy, 100.0);
    assert!(!r.summary.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_and_black_to_move_start() {
    // Black king in check with exactly one legal move (Kh7 is attacked... use a simple
    // position): black king h8, white queen g6 & king f6? Keep it simple: start with black to
    // move from a custom FEN where black has a single legal move.
    let content = Arc::new(gm_content::Content::default());
    // Black: Kh8, pawn h7 blocked; White rook on g1 controls g-file, king f7? Construct:
    // White Ka1, Rg1; Black Kh8, Ph7, Ph6(?) -> legal moves for black: Kh8 has g8? attacked by
    // rook g1; g7 attacked by rook g1; h7 own pawn. Pawn h7 blocked by h6 pawn (black). h6 pawn
    // can push h5. So 1 legal move: h6h5.
    let fen = "7k/7p/7p/8/8/8/8/K5R1 b - - 0 1";
    let review = review_game(&pool(), content, fen, &ucis("h6h5 g1g2"), 6, None, Lang::En).await.expect("review");
    assert_eq!(review.moves[0].classification, Classification::Forced);
    assert_eq!(review.moves[0].color, "black");
    assert_eq!(review.moves[1].color, "white");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_positions_are_consistent() {
    let content = Arc::new(gm_content::Content::default());
    let moves = ucis("g1f3 g8f6 f3g1 f6g8 g1f3 g8f6 f3g1 f6g8");
    let review = review_game(&pool(), content, "start", &moves, 6, None, Lang::En).await.expect("review");
    assert_eq!(review.evals[0], review.evals[4]);
    assert_eq!(review.evals[1], review.evals[5]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_review_does_not_hang_pool() {
    let content = Arc::new(gm_content::Content::default());
    let p = pool();
    let moves = ucis("e2e4 e7e5 g1f3 b8c6 f1b5 a7a6 b5a4 g8f6 e1g1 f8e7");
    let fut = review_game(&p, content.clone(), "start", &moves, 30, None, Lang::En);
    // Cancel almost immediately.
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), fut).await;
    // The pool must become fully available again quickly (searches stopped).
    let t0 = std::time::Instant::now();
    while p.available() < p.size() {
        assert!(t0.elapsed() < std::time::Duration::from_secs(10), "pool still busy");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[test]
fn sacrifice_is_brilliant() {
    // White plays Bc4 into a pawn capture (dxc4 wins a bishop by SEE), engine says it's best.
    let before_pos = gm_engine::parse_fen("4k3/8/8/3p4/8/8/8/4KB2 w - - 0 1").expect("fen");
    let mv = uci_to_move(&before_pos, "f1c4").expect("move");
    let mut after_pos = before_pos.clone();
    after_pos.play_unchecked(&mv);
    let line = |uci: &str, cp: i32| PvLine { score: Score::Cp(cp), moves: vec![uci.to_string()], san: vec![] };
    let before = PosData {
        fen: to_fen(&before_pos),
        pos: before_pos,
        score: Score::Cp(60),
        white_win: win_percent(Score::Cp(60)),
        lines: vec![line("f1c4", 60), line("f1e2", 50)],
    };
    let after = PosData {
        fen: to_fen(&after_pos),
        pos: after_pos,
        score: Score::Cp(60),
        white_win: win_percent(Score::Cp(60)),
        lines: vec![],
    };
    let facts = MoveFacts {
        played_uci: "f1c4",
        mv: &mv,
        before: &before,
        after: &after,
        mover: Color::White,
        legal_count: 10,
        is_book: false,
        win_before: before.white_win,
        win_after: after.white_win,
        loss: 0.0,
        prev_opponent_loss: 0.0,
        prev_capture_square: None,
    };
    assert_eq!(classify(&facts), Classification::Brilliant);

    // Same move when already totally winning is just "best".
    let facts2 = MoveFacts { win_before: 97.0, win_after: 97.0, ..facts };
    assert_ne!(classify(&facts2), Classification::Brilliant);
}

#[test]
fn miss_after_opponent_error() {
    let before_pos = gm_engine::parse_fen("4k3/8/8/3q4/8/8/8/3RK3 w - - 0 1").expect("fen");
    let mv = uci_to_move(&before_pos, "e1f2").expect("move");
    let mut after_pos = before_pos.clone();
    after_pos.play_unchecked(&mv);
    let before = PosData {
        fen: to_fen(&before_pos),
        pos: before_pos,
        score: Score::Cp(800),
        white_win: win_percent(Score::Cp(800)),
        lines: vec![
            PvLine { score: Score::Cp(800), moves: vec!["d1d5".into()], san: vec!["Rxd5".into()] },
            PvLine { score: Score::Cp(0), moves: vec!["e1f2".into()], san: vec![] },
        ],
    };
    let after = PosData {
        fen: to_fen(&after_pos),
        pos: after_pos,
        score: Score::Cp(0),
        white_win: 50.0,
        lines: vec![],
    };
    let wb = before.white_win;
    let facts = MoveFacts {
        played_uci: "e1f2",
        mv: &mv,
        before: &before,
        after: &after,
        mover: Color::White,
        legal_count: 15,
        is_book: false,
        win_before: wb,
        win_after: 50.0,
        loss: wb - 50.0,
        prev_opponent_loss: 40.0,
        prev_capture_square: None,
    };
    assert_eq!(classify(&facts), Classification::Miss);
    let facts2 = MoveFacts { prev_opponent_loss: 0.0, ..facts };
    assert_eq!(classify(&facts2), Classification::Blunder);
}

#[test]
fn win_percent_shape() {
    assert!((win_percent(Score::Cp(0)) - 50.0).abs() < 1e-3);
    assert!(win_percent(Score::Cp(300)) > 70.0);
    assert!(win_percent(Score::Cp(-300)) < 30.0);
    assert!(win_percent(Score::Mate(3)) > 97.0);
    assert!(win_percent(Score::Mate(-3)) < 3.0);
    assert_eq!(win_percent(Score::Cp(5000)), win_percent(Score::Cp(1000)));
}

/// Perf check: `cargo test -p gm-analysis --release -- --ignored --nocapture perf`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn perf_40_move_game_depth_12() {
    let content = Arc::new(gm_content::Content::default());
    // Build an 80-ply game by quick self-play so the move list is always legal.
    let mut engine = gm_engine::Engine::new(16);
    let mut pos = Chess::default();
    let mut moves = Vec::new();
    let stop = AtomicBool::new(false);
    while moves.len() < 80 && !pos.is_game_over() {
        let info = engine.search(&pos, &SearchLimits { depth: Some(4), ..Default::default() }, &stop, &mut |_| {});
        let Some(u) = info.best_move().map(str::to_string) else { break };
        let m = uci_to_move(&pos, &u).expect("engine move");
        pos.play_unchecked(&m);
        moves.push(u);
    }
    let n = num_cpus();
    let p = EnginePool::new(n, 32);
    let t0 = std::time::Instant::now();
    let r = review_game(&p, content, "start", &moves, 12, None, Lang::En).await.expect("review");
    eprintln!(
        "reviewed {} plies at depth 12 with {n} engines in {:?}; acc W {:.1} B {:.1}\n{}",
        r.moves.len(),
        t0.elapsed(),
        r.white.accuracy,
        r.black.accuracy,
        r.summary
    );
}

fn num_cpus() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

fn fixture_content() -> Arc<gm_content::Content> {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../gm-content/tests/fixtures/data");
    Arc::new(gm_content::Content::load(&dir).expect("fixture content"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spanish_review_text() {
    let content = fixture_content();
    let moves = ucis("e2e4 e7e5 g1f3 b8c6 f1c4 f8c5 c4f7 e8f7 f3e5 c6e5");
    let r = review_game(&pool(), content, "start", &moves, 6, None, Lang::Es).await.expect("review");
    assert_eq!(r.opening.as_ref().map(|o| o.name.as_str()), Some("Giuoco piano"));
    assert_eq!(r.moves[5].opening_name.as_deref(), Some("Giuoco piano"));
    assert!(r.summary.contains("Precisión: blancas"), "{}", r.summary);
    assert!(r.summary.contains("Tras la apertura (Giuoco piano)"), "{}", r.summary);
    assert!(!r.summary.contains("White") && !r.summary.contains("Accuracy"), "{}", r.summary);
    // Classification values stay machine keys.
    assert_eq!(serde_json::to_value(r.moves[0].classification).expect("json"), serde_json::json!("book"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relocalize_rewrites_text_only_without_engine() {
    let content = fixture_content();
    let moves = ucis("e2e4 e7e5 g1f3 b8c6 f1c4 f8c5 c4f7 e8f7 f3e5 c6e5");
    let en = review_game(&pool(), Arc::clone(&content), "start", &moves, 8, None, Lang::En).await.expect("review");

    // `relocalize` is a plain sync function with no engine or pool in reach: it cannot search.
    // It is also orders of magnitude faster than any review.
    let t0 = std::time::Instant::now();
    let es = relocalize(&en, &content, Lang::Es);
    let took = t0.elapsed();
    assert!(took < std::time::Duration::from_millis(500), "relocalize took {took:?}");

    // Everything but text is identical.
    assert_eq!(es.evals, en.evals);
    assert_eq!(es.key_moments, en.key_moments);
    assert_eq!(es.white, en.white);
    assert_eq!(es.black, en.black);
    assert_eq!(es.start_fen, en.start_fen);
    assert_eq!(es.opening.as_ref().map(|o| &o.id), en.opening.as_ref().map(|o| &o.id));
    assert_eq!(es.moves.len(), en.moves.len());
    for (a, b) in en.moves.iter().zip(&es.moves) {
        assert_eq!((a.ply, &a.san, &a.uci, &a.color, a.classification), (b.ply, &b.san, &b.uci, &b.color, b.classification));
        assert_eq!((a.eval_before, a.eval_after, &a.best_move_uci, &a.best_line_san), (b.eval_before, b.eval_after, &b.best_move_uci, &b.best_line_san));
        assert_eq!(a.win_chance_loss, b.win_chance_loss);
        assert_eq!(a.opening_name.is_some(), b.opening_name.is_some());
        assert_ne!(a.explanation, b.explanation, "ply {} not translated: {}", a.ply, b.explanation);
    }
    assert_ne!(es.summary, en.summary);
    assert!(es.summary.contains("Precisión"), "{}", es.summary);
    assert_eq!(es.opening.as_ref().map(|o| o.name.as_str()), Some("Giuoco piano"));
    // Bxf7+ gives up the bishop: explained in Spanish.
    let bxf7 = &es.moves[6];
    assert!(!bxf7.explanation.contains("This") && !bxf7.explanation.contains(" was "), "{}", bxf7.explanation);

    // Round trip back to English reproduces the original text exactly.
    assert_eq!(relocalize(&es, &content, Lang::En), en);
}

#[test]
fn stored_json_round_trip() {
    let r = GameReview { summary: "Hola".into(), ..Default::default() };
    let j = to_stored_json(&r, Lang::Es).expect("json");
    assert!(j.contains("\"lang\":\"es\""), "{j}");
    let (back, lang) = from_stored_json(&j).expect("parse");
    assert_eq!((back, lang), (r.clone(), Lang::Es));
    // Legacy rows without a tag are English.
    let legacy = serde_json::to_string(&r).expect("json");
    assert_eq!(from_stored_json(&legacy).map(|(_, l)| l), Some(Lang::En));
    assert!(from_stored_json("not json").is_none());
}
