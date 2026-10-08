//! Analyse a FEN: `cargo run --release -p gm-engine --example analyze -- "<fen>" [depth] [multipv]`
use gm_engine::*;
use std::sync::atomic::AtomicBool;
fn main() {
    let fen = std::env::args().nth(1).unwrap();
    let d: u8 = std::env::args()
        .nth(2)
        .and_then(|x| x.parse().ok())
        .unwrap_or(10);
    let mpv: usize = std::env::args()
        .nth(3)
        .and_then(|x| x.parse().ok())
        .unwrap_or(1);
    let pos = parse_fen(&fen).unwrap();
    let mut e = Engine::new(16);
    let stop = AtomicBool::new(false);
    e.search(
        &pos,
        &SearchLimits {
            depth: Some(d),
            multipv: mpv,
            ..Default::default()
        },
        &stop,
        &mut |i| {
            for l in &i.lines {
                println!("d{} {} {}", i.depth, l.score, l.san.join(" "));
            }
        },
    );
}
