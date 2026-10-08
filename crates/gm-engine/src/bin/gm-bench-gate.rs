//! Engine quality gate: `cargo run --release -p gm-engine --bin gm-bench-gate [--quick]`.
//!
//! Prints one JSON object on stdout:
//! - `speed`: fixed-depth searches of reference positions → total nodes, time and nodes/s;
//! - `tactics`: fixed-depth searches of tactical positions, each with the expected best move(s).
//!
//! Every search is deterministic in its node count (fixed depth, fresh tables), so only the
//! time varies between machines. `tools/qa/bench-gate.mjs` compares the result with
//! `tools/qa/bench-baseline.json`.

use std::sync::atomic::AtomicBool;

use gm_engine::{parse_fen, Engine, SearchLimits};
use serde_json::{json, Value};

/// (name, FEN, depth) — searched for raw speed.
const SPEED: &[(&str, &str, u8)] = &[
    (
        "startpos",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        18,
    ),
    (
        "kiwipete",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        15,
    ),
    (
        "middlegame",
        "r1bq1rk1/pp2bppp/2n1pn2/3p4/2PP4/2N1PN2/PP1B1PPP/R2QKB1R w KQ - 0 8",
        17,
    ),
    ("endgame", "8/8/4k3/3p4/3P4/4K3/8/8 w - - 0 1", 30),
];

/// (name, FEN, depth, accepted best moves in UCI).
const TACTICS: &[(&str, &str, u8, &[&str])] = &[
    (
        "back-rank-mate",
        "6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1",
        4,
        &["d1d8"],
    ),
    (
        "scholars-mate",
        "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
        4,
        &["f3f7"],
    ),
    (
        "win-hanging-queen",
        "rnb1kbnr/pppp1ppp/8/4p1q1/3P4/2N5/PPP1PPPP/R1BQKBNR w KQkq - 0 3",
        5,
        &["c1g5"],
    ),
    (
        "knight-fork",
        "r3k3/8/8/1N6/8/8/4P3/4K3 w - - 0 1",
        5,
        &["b5c7"],
    ),
    (
        "smothered-mate",
        "6rk/6pp/8/6N1/8/8/8/6K1 w - - 0 1",
        4,
        &["g5f7"],
    ),
    (
        "legals-mate-in-2",
        "r2qkb1r/pp2nppp/3p4/2pNN1B1/2BnP3/3P4/PPP2PPP/R2bK2R w KQkq - 1 1",
        6,
        &["d5f6"],
    ),
];

fn search(engine: &mut Engine, fen: &str, depth: u8) -> Result<gm_engine::SearchInfo, String> {
    let pos = parse_fen(fen)?;
    engine.new_game();
    let stop = AtomicBool::new(false);
    let limits = SearchLimits {
        depth: Some(depth),
        ..Default::default()
    };
    Ok(engine.search(&pos, &limits, &stop, &mut |_| {}))
}

fn main() {
    let quick = std::env::args().any(|a| a == "--quick");
    let mut engine = Engine::new(64);

    let mut speed = Vec::new();
    let (mut total_nodes, mut total_ms) = (0u64, 0u64);
    for &(name, fen, depth) in SPEED {
        let depth = if quick {
            depth.saturating_sub(2).max(1)
        } else {
            depth
        };
        match search(&mut engine, fen, depth) {
            Ok(info) => {
                total_nodes += info.nodes;
                total_ms += info.time_ms;
                speed.push(json!({
                    "name": name, "depth": info.depth, "nodes": info.nodes,
                    "time_ms": info.time_ms, "best": info.best_move(),
                }));
            }
            Err(e) => speed.push(json!({ "name": name, "error": e })),
        }
    }

    let mut tactics = Vec::new();
    for &(name, fen, depth, expected) in TACTICS {
        let entry: Value = match search(&mut engine, fen, depth) {
            Ok(info) => {
                let best = info.best_move().unwrap_or("").to_string();
                json!({
                    "name": name, "fen": fen, "depth": depth, "best": best,
                    "expected": expected, "solved": expected.contains(&best.as_str()),
                    "score": info.best().map(|l| l.score.to_string()),
                })
            }
            Err(e) => json!({ "name": name, "fen": fen, "error": e, "solved": false }),
        };
        tactics.push(entry);
    }

    let out = json!({
        "quick": quick,
        "speed": {
            "positions": speed,
            "total_nodes": total_nodes,
            "total_ms": total_ms,
            "nps": total_nodes * 1000 / total_ms.max(1),
        },
        "tactics": tactics,
    });
    println!("{out}");
}
