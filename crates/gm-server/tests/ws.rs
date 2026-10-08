//! WebSocket protocol test against a real listener.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

async fn start() -> (
    std::net::SocketAddr,
    gm_engine::EnginePool,
    tokio::sync::watch::Receiver<bool>,
    Arc<tokio::sync::watch::Sender<bool>>,
) {
    let pool = gm_engine::EnginePool::new(2, 4);
    let state = gm_server::AppState::new(
        Arc::new(gm_content::Content::default()),
        pool.clone(),
        gm_store::Store::open_in_memory().unwrap(),
        Arc::new(gm_mentor::Mentor::from_env()),
    );
    let sd = Arc::clone(&state.shutdown);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = gm_server::app(state, std::path::Path::new("/nonexistent"));
    tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, pool, sd.subscribe(), sd)
}

async fn next_json<S>(ws: &mut S) -> Value
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let m = tokio::time::timeout(Duration::from_secs(40), ws.next())
            .await
            .expect("timeout")
            .expect("closed")
            .expect("ws error");
        if let Message::Text(t) = m {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn analyze_supersede_stop_and_close() {
    let (addr, pool, _rx, sd) = start().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/engine/ws"))
        .await
        .unwrap();

    // bad fen -> error
    ws.send(Message::Text(
        json!({"type":"analyze","id":1,"fen":"nope"}).to_string(),
    ))
    .await
    .unwrap();
    let v = next_json(&mut ws).await;
    assert_eq!(v["type"], "error");
    assert_eq!(v["id"], 1);

    // long analysis, then supersede it immediately with a short one
    ws.send(Message::Text(
        json!({"type":"analyze","id":2,"fen":"startpos","movetime_ms":20000,"multipv":2})
            .to_string(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t0 = Instant::now();
    ws.send(Message::Text(
        json!({"type":"analyze","id":3,"fen":"startpos","movetime_ms":500}).to_string(),
    ))
    .await
    .unwrap();
    let mut infos = 0;
    loop {
        let v = next_json(&mut ws).await;
        if v["id"] == 3 {
            if v["type"] == "info" {
                infos += 1;
            }
            if v["type"] == "done" {
                assert!(v["lines"].is_array());
                break;
            }
        }
    }
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "superseded search did not stop"
    );
    assert!(infos <= 7, "info not throttled: {infos}");

    // stop -> done arrives quickly
    ws.send(Message::Text(
        json!({"type":"analyze","id":4,"fen":"startpos","movetime_ms":20000}).to_string(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let t0 = Instant::now();
    ws.send(Message::Text(json!({"type":"stop"}).to_string()))
        .await
        .unwrap();
    loop {
        let v = next_json(&mut ws).await;
        if v["id"] == 4 && v["type"] == "done" {
            break;
        }
    }
    assert!(t0.elapsed() < Duration::from_secs(3));

    // close mid-search -> engines return to the pool
    ws.send(Message::Text(
        json!({"type":"analyze","id":5,"fen":"startpos","movetime_ms":20000}).to_string(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(ws);
    let t0 = Instant::now();
    while pool.available() < pool.size() {
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "engine leaked after socket close"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // shutdown closes open sockets
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/engine/ws"))
        .await
        .unwrap();
    sd.send_replace(true);
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(m)) = ws.next().await {
            if matches!(m, Message::Close(_)) {
                return true;
            }
        }
        true
    })
    .await
    .unwrap_or(false);
    assert!(closed);
}
