//! Engine benchmark: `cargo run --release -p gm-engine --bin gm-bench [movetime_ms]`.
//! Prints per-iteration depth / score / nodes / nps / PV for a few reference positions.

use std::sync::atomic::AtomicBool;

use gm_engine::{parse_fen, Engine, SearchLimits};

const POSITIONS: &[(&str, &str)] = &[
    (
        "startpos",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    ),
    (
        "kiwipete",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    ),
    (
        "middlegame",
        "r1bq1rk1/pp2bppp/2n1pn2/3p4/2PP4/2N1PN2/PP1B1PPP/R2QKB1R w KQ - 0 8",
    ),
    ("endgame", "8/8/4k3/3p4/3P4/4K3/8/8 w - - 0 1"),
];

fn main() {
    let movetime: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1000);
    let mut engine = Engine::new(64);
    let stop = AtomicBool::new(false);
    let mut total_nodes = 0u64;
    let mut total_ms = 0u64;
    for (name, fen) in POSITIONS {
        let pos = match parse_fen(fen) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{name}: {e}");
                continue;
            }
        };
        engine.new_game();
        println!("== {name} ({movetime} ms)");
        let limits = SearchLimits {
            movetime_ms: Some(movetime),
            ..Default::default()
        };
        let info = engine.search(&pos, &limits, &stop, &mut |i| {
            let line = i.lines.first();
            println!(
                "  depth {:>2} sel {:>2}  {:>7}  nodes {:>9}  nps {:>8}  {:>5} ms  {}",
                i.depth,
                i.seldepth,
                line.map(|l| l.score.to_string()).unwrap_or_default(),
                i.nodes,
                i.nps,
                i.time_ms,
                line.map(|l| l.san.join(" ")).unwrap_or_default()
            );
        });
        total_nodes += info.nodes;
        total_ms += info.time_ms;
        println!(
            "  => best {} depth {}",
            info.best_move().unwrap_or("-"),
            info.depth
        );
    }
    println!(
        "total nodes {total_nodes}  nps {}",
        total_nodes * 1000 / total_ms.max(1)
    );
}
