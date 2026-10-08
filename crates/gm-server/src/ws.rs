//! `GET /api/engine/ws` — live analysis for the eval bar / engine lines.
//!
//! One task per socket. Each socket runs at most one search at a time: a new `analyze`
//! (or `stop`, or the socket closing, or server shutdown) raises the previous search's
//! `AtomicBool` stop flag so the engine returns to the pool promptly. `info` messages are
//! throttled to <= 10/s; the final result is sent as `done`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};

use gm_engine::{parse_fen, SearchInfo};

use crate::api::clamp_limits;
use crate::state::AppState;

/// Server-side cap for websocket searches.
pub const WS_MAX_MOVETIME_MS: u64 = 30_000;
/// Default when the client gives neither depth nor movetime.
const WS_DEFAULT_MOVETIME_MS: u64 = 3_000;
/// Minimum spacing of `info` messages (=> at most 10/s).
const INFO_INTERVAL: Duration = Duration::from_millis(100);
/// Inbound message cap.
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
/// Outbound queue per socket; `info` is dropped (never blocks the engine) when full.
const OUT_QUEUE: usize = 32;
/// Close idle sockets that send nothing (not even pongs) for this long.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClientMsg {
    #[serde(rename = "type")]
    kind: String,
    id: Value,
    fen: String,
    multipv: Option<usize>,
    movetime_ms: Option<u64>,
    depth: Option<u8>,
    nodes: Option<u64>,
}

pub async fn engine_ws(State(st): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(st, socket))
}

/// `{"type": kind, "id": id, ...SearchInfo}`
fn info_message(kind: &str, id: &Value, info: &SearchInfo) -> String {
    let mut v = serde_json::to_value(info).unwrap_or_else(|_| json!({}));
    if let Value::Object(map) = &mut v {
        map.insert("type".into(), Value::String(kind.into()));
        map.insert("id".into(), id.clone());
    }
    v.to_string()
}

fn error_message(id: &Value, error: &str) -> String {
    json!({ "type": "error", "id": id, "error": error }).to_string()
}

/// The search currently running on this socket.
struct Running {
    stop: Arc<AtomicBool>,
    abort: AbortHandle,
}

impl Running {
    /// Stop the search; `abort` also drops it if it is still waiting for a free engine.
    fn cancel(&self, abort: bool) {
        self.stop.store(true, Ordering::Relaxed);
        if abort {
            self.abort.abort();
        }
    }
}

async fn handle_socket(st: AppState, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(OUT_QUEUE);

    // Writer: the only owner of the sink. Ends when every sender is gone or the peer is gone.
    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            let closing = matches!(msg, Message::Close(_));
            if sink.send(msg).await.is_err() || closing {
                break;
            }
        }
        let _ = sink.close().await;
    });

    let mut tasks: JoinSet<()> = JoinSet::new();
    let mut current: Option<Running> = None;
    let mut shutdown = st.shutdown.subscribe();
    tracing::debug!("engine websocket connected");

    loop {
        let next = tokio::select! {
            msg = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => msg,
            _ = shutdown.wait_for(|v| *v) => {
                let _ = out_tx.try_send(Message::Close(Some(CloseFrame { code: 1001, reason: "server shutting down".into() })));
                break;
            }
            // Reap finished searches so the JoinSet never grows.
            Some(_) = tasks.join_next(), if !tasks.is_empty() => continue,
        };
        let msg = match next {
            Err(_) => {
                tracing::debug!("engine websocket idle timeout");
                break;
            }
            Ok(None) | Ok(Some(Err(_))) => break,
            Ok(Some(Ok(m))) => m,
        };
        let text = match msg {
            Message::Text(t) => t,
            Message::Binary(_) => {
                let _ = out_tx.try_send(Message::Text(error_message(
                    &Value::Null,
                    "binary messages are not supported",
                )));
                continue;
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue, // pings are answered automatically
        };
        let cmd: ClientMsg = match serde_json::from_str(&text) {
            Ok(c) => c,
            Err(e) => {
                let _ = out_tx.try_send(Message::Text(error_message(
                    &Value::Null,
                    &format!("invalid message: {e}"),
                )));
                continue;
            }
        };
        match cmd.kind.as_str() {
            "analyze" => {
                if let Some(prev) = current.take() {
                    prev.cancel(true);
                }
                let pos = match parse_fen(&cmd.fen) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = out_tx.try_send(Message::Text(error_message(&cmd.id, &e)));
                        continue;
                    }
                };
                let limits = clamp_limits(
                    cmd.depth,
                    cmd.movetime_ms,
                    cmd.nodes,
                    cmd.multipv,
                    WS_MAX_MOVETIME_MS,
                    WS_DEFAULT_MOVETIME_MS,
                );
                let stop = Arc::new(AtomicBool::new(false));
                let abort = tasks.spawn(run_analysis(
                    st.pool.clone(),
                    pos,
                    limits,
                    Arc::clone(&stop),
                    cmd.id,
                    out_tx.clone(),
                ));
                current = Some(Running { stop, abort });
            }
            "stop" => {
                // Let the task send its `done` with the best result so far.
                if let Some(prev) = current.take() {
                    prev.cancel(false);
                }
            }
            "ping" => {
                let _ = out_tx.try_send(Message::Text(
                    json!({ "type": "pong", "id": cmd.id }).to_string(),
                ));
            }
            other => {
                let _ = out_tx.try_send(Message::Text(error_message(
                    &cmd.id,
                    &format!("unknown message type {other:?}"),
                )));
            }
        }
    }

    // Socket closed / shutdown: stop everything, wait for the engines to be released.
    if let Some(prev) = current.take() {
        prev.cancel(true);
    }
    tasks.shutdown().await;
    drop(out_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
    tracing::debug!("engine websocket closed");
}

async fn run_analysis(
    pool: gm_engine::EnginePool,
    pos: shakmaty::Chess,
    limits: gm_engine::SearchLimits,
    stop: Arc<AtomicBool>,
    id: Value,
    out: mpsc::Sender<Message>,
) {
    let (stop2, id2, out2) = (Arc::clone(&stop), id.clone(), out.clone());
    let pool2 = pool.clone();
    // Spawned so an engine panic cannot take down the socket task.
    let result = tokio::spawn(async move {
        pool2
            .with_engine(move |engine| {
                if stop2.load(Ordering::Relaxed) {
                    return None; // cancelled while waiting for an engine
                }
                let mut last_sent: Option<Instant> = None;
                let info = engine.search(&pos, &limits, &stop2, &mut |info| {
                    if stop2.load(Ordering::Relaxed) || info.lines.is_empty() {
                        return;
                    }
                    let now = Instant::now();
                    if last_sent.is_none_or(|t| now.duration_since(t) >= INFO_INTERVAL) {
                        last_sent = Some(now);
                        // Never block the search thread: drop the update if the client is slow.
                        let _ = out2.try_send(Message::Text(info_message("info", &id2, info)));
                    }
                });
                Some(info)
            })
            .await
    });
    // If this task is aborted, make sure the search (which may be mid-flight on a blocking
    // thread) is told to stop as well.
    struct Guard(Arc<AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let _guard = Guard(Arc::clone(&stop));
    match result.await {
        Ok(Some(info)) => {
            let _ = out
                .send(Message::Text(info_message("done", &id, &info)))
                .await;
        }
        Ok(None) => {}
        Err(e) => {
            if e.is_panic() {
                tracing::error!("engine panicked during websocket analysis");
                let _ = out
                    .send(Message::Text(error_message(&id, "engine error")))
                    .await;
            }
        }
    }
}
