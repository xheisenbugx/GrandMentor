//! Tactical sanity suite (WAC 11-20 + WAC 2): `cargo run --release -p gm-engine --example wac`
use gm_engine::*;
use std::sync::atomic::AtomicBool;
fn main() {
    let cases = [
        (
            "r1b1kb1r/3q1ppp/pBp1pn2/8/Np3P2/5B2/PPP3PP/R2Q1RK1 w kq - 0 1",
            "f3c6",
        ),
        (
            "4k1r1/2p3r1/1pR1p3/3pP2p/3P2qP/P4N2/1PQ4P/5R1K b - - 0 1",
            "g4f3",
        ),
        (
            "5rk1/pp4p1/2n1p2p/2Npq3/2p5/6P1/P3P1BP/R4Q1K w - - 0 1",
            "f1f8",
        ),
        (
            "r2rb1k1/pp1q1p1p/2n1p1p1/2bp4/5P2/PP1BPR1Q/1BPN2PP/R5K1 w - - 0 1",
            "h3h7",
        ),
        (
            "1R6/1brk2p1/4p2p/p1P1Pp2/P7/6P1/1P4P1/2R3K1 w - - 0 1",
            "b8b7",
        ),
        (
            "r4rk1/ppp2ppp/2n5/2bqp3/8/P2PB3/1PP1NPPP/R2Q1RK1 w - - 0 1",
            "e2c3",
        ),
        ("R7/P4k2/8/8/8/8/r7/6K1 w - - 0 1", "a8h8"),
        (
            "r1b2rk1/ppbn1ppp/4p3/1QP4q/3P4/N4N2/5PPP/R1B2RK1 w - - 0 1",
            "c5c6",
        ),
        (
            "r2qkb1r/1ppb1ppp/p7/4p3/P1Q1P3/2P5/5PPP/R1B2KNR b kq - 0 1",
            "d7b5",
        ),
        ("8/7p/5k2/5p2/p1p2P2/Pr1pPK2/1P1R3P/8 b - - 0 1", "b3b2"),
    ];
    let mut e = Engine::new(64);
    let stop = AtomicBool::new(false);
    let mut ok = 0;
    for (f, bm) in cases {
        e.new_game();
        let pos = parse_fen(f).unwrap();
        let i = e.search(
            &pos,
            &SearchLimits {
                movetime_ms: Some(1000),
                ..Default::default()
            },
            &stop,
            &mut |_| {},
        );
        let b = i.best_move().unwrap_or("-");
        if b == bm {
            ok += 1
        }
        println!(
            "{} {} want {} got {} d{} {}",
            if b == bm { "OK  " } else { "FAIL" },
            f,
            bm,
            b,
            i.depth,
            i.score()
        );
    }
    println!("{ok}/10");
}
