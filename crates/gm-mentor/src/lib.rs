//! gm-mentor: the GrandMentor coach, "Mentor Mira".
//!
//! * [`explain_move`] — instant, rule-based, beginner-friendly explanation of a move
//!   (hanging pieces, forks, pins, skewers, mates, development, castling, center, trades...).
//! * [`describe_position`] — 3-6 short ideas/plans for a position.
//! * [`Mentor::chat`] — answers questions about a position. Uses the Anthropic Messages API when
//!   `ANTHROPIC_API_KEY` is set (model `GM_MENTOR_MODEL`, default `claude-opus-5-5`), and falls
//!   back to the rule-based coach on any failure or when no key is configured.
//!
//! All functions are panic-free on arbitrary input and allocate only bounded amounts of memory.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

pub use gm_engine::Score;

mod coach;
mod describe;
mod explain;
mod llm;
mod phrase;
pub mod tactics;

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Display name of the mentor persona.
pub const MENTOR_NAME: &str = "Mentor Mira";
/// Maximum concurrent LLM requests per process; extra requests get the instant coach answer.
const MAX_CONCURRENT_LLM: usize = 4;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct MoveContext {
    pub fen_before: String,
    pub played_uci: String,
    pub played_san: String,
    pub best_uci: String,
    pub best_san: String,
    pub best_line_san: Vec<String>,
    pub eval_before: Score,
    pub eval_after: Score,
    pub classification: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ChatTurn {
    /// user | mentor
    pub role: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ChatRequest {
    pub question: String,
    pub fen: String,
    pub moves_san: Vec<String>,
    /// e.g. "+0.45: Nf3 Nc6 Bb5"
    pub engine_lines: Vec<String>,
    pub history: Vec<ChatTurn>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ChatResponse {
    pub answer: String,
    /// "llm" | "coach"
    pub source: String,
}

/// Rule-based, instant, friendly, 1-3 sentences.
pub fn explain_move(ctx: &MoveContext) -> String {
    explain::explain_move(ctx)
}

/// Plans / features of a position: material, king safety, open files, hanging pieces...
pub fn describe_position(fen: &str) -> Vec<String> {
    describe::describe_position(fen)
}

/// The rule-based chat answer (no network). Exposed for tests and offline use.
pub fn coach_answer(req: &ChatRequest) -> ChatResponse {
    ChatResponse { answer: coach::answer(req), source: "coach".to_string() }
}

/// Holds an HTTP client + optional API key. Cheap to clone.
#[derive(Clone)]
pub struct Mentor {
    client: reqwest::Client,
    api_key: Option<Arc<str>>,
    model: String,
    endpoint: String,
    permits: Arc<Semaphore>,
}

impl std::fmt::Debug for Mentor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mentor").field("llm_enabled", &self.llm_enabled()).field("model", &self.model).finish()
    }
}

impl Mentor {
    /// Reads `ANTHROPIC_API_KEY` (optional) and `GM_MENTOR_MODEL` (default `claude-opus-5-5`).
    pub fn from_env() -> Self {
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
        let model = std::env::var("GM_MENTOR_MODEL").ok();
        Self::new(api_key, model)
    }

    /// Explicit configuration (blank values are treated as absent).
    pub fn new(api_key: Option<String>, model: Option<String>) -> Self {
        let api_key = api_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).map(Arc::from);
        let model = model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty() && m.len() <= 100)
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let client = reqwest::Client::builder()
            .timeout(llm::REQUEST_TIMEOUT + Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(4)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Mentor { client, api_key, model, endpoint: llm::API_URL.to_string(), permits: Arc::new(Semaphore::new(MAX_CONCURRENT_LLM)) }
    }

    pub fn llm_enabled(&self) -> bool {
        self.api_key.is_some()
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Answers a question; falls back to the rule-based coach when the LLM is unavailable.
    pub async fn chat(&self, req: ChatRequest) -> ChatResponse {
        let req = sanitize(req);
        if req.question.trim().is_empty() {
            return ChatResponse {
                answer: format!("Hi, I'm {MENTOR_NAME}! Ask me anything about this position — the plan, the best move, or why a move was good or bad."),
                source: "coach".into(),
            };
        }
        if let Some(key) = &self.api_key {
            match self.try_llm(key, &req).await {
                Ok(answer) => return ChatResponse { answer, source: "llm".into() },
                Err(e) => tracing::warn!(error = %e, model = %self.model, "mentor LLM call failed; using rule-based coach"),
            }
        }
        // The rule-based coach is CPU-only and fast (microseconds to a few ms).
        coach_answer(&req)
    }

    async fn try_llm(&self, key: &str, req: &ChatRequest) -> Result<String, llm::LlmError> {
        let _permit = tokio::time::timeout(Duration::from_secs(5), self.permits.acquire())
            .await
            .map_err(|_| llm::LlmError::Other("mentor is busy".into()))?
            .map_err(|_| llm::LlmError::Other("mentor shutting down".into()))?;
        match llm::call(&self.client, &self.endpoint, key, &self.model, req, true).await {
            Err(llm::LlmError::BadRequest(msg)) => {
                tracing::debug!(%msg, "retrying mentor LLM call without optional parameters");
                llm::call(&self.client, &self.endpoint, key, &self.model, req, false).await
            }
            other => other,
        }
    }
}

/// Bound every field so a hostile/buggy client can't make us build huge prompts.
fn sanitize(mut req: ChatRequest) -> ChatRequest {
    fn cap(s: &mut String, max: usize) {
        if s.len() > max {
            let mut end = max;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
    }
    cap(&mut req.question, 4000);
    cap(&mut req.fen, 200);
    if req.moves_san.len() > 600 {
        let drop = req.moves_san.len() - 600;
        req.moves_san.drain(..drop);
    }
    for m in &mut req.moves_san {
        cap(m, 12);
    }
    req.engine_lines.truncate(8);
    for l in &mut req.engine_lines {
        cap(l, 400);
    }
    if req.history.len() > 20 {
        let drop = req.history.len() - 20;
        req.history.drain(..drop);
    }
    for t in &mut req.history {
        cap(&mut t.role, 16);
        cap(&mut t.text, 4000);
    }
    req
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(q: &str, fen: &str, lines: &[&str]) -> ChatRequest {
        ChatRequest {
            question: q.into(),
            fen: fen.into(),
            engine_lines: lines.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    const ITALIAN: &str = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3";

    #[tokio::test]
    async fn chat_falls_back_without_key() {
        let m = Mentor::new(None, None);
        assert!(!m.llm_enabled());
        assert_eq!(m.model(), DEFAULT_MODEL);
        let r = m.chat(req("What should I do?", ITALIAN, &["+0.25: Bc5 c3 Nf6"])).await;
        assert_eq!(r.source, "coach");
        assert!(r.answer.contains("Bc5"), "{}", r.answer);
    }

    #[tokio::test]
    async fn chat_with_bad_key_falls_back() {
        // A key is set but the request is invalid: we must still answer from the coach.
        // Point at a closed local port so the test never touches the network.
        let mut m = Mentor::new(Some("sk-invalid".into()), Some("claude-opus-5-5".into()));
        m.endpoint = "http://127.0.0.1:9/v1/messages".into();
        assert!(m.llm_enabled());
        let r = m.chat(req("plan?", ITALIAN, &[])).await;
        assert_eq!(r.source, "coach");
        assert!(!r.answer.is_empty());
    }

    #[test]
    fn coach_routes_keywords() {
        let plan = coach_answer(&req("What's the plan here?", ITALIAN, &["+0.25: Bc5 c3 Nf6"]));
        assert!(plan.answer.contains('•'), "{}", plan.answer);
        let eval = coach_answer(&req("Who is winning?", ITALIAN, &["+0.25: Bc5 c3 Nf6"]));
        assert!(eval.answer.contains("roughly equal"), "{}", eval.answer);
        // Hypothetical move: ...Nd4 is fine, but ...Qh4 just hangs nothing; ...Ba3 hangs the bishop.
        let why = coach_answer(&req("Why is Ba3 bad?", ITALIAN, &["+0.25: Bc5 c3 Nf6"]));
        assert!(why.answer.contains("bishop on a3"), "{}", why.answer);
        assert!(why.answer.contains("Bc5"), "{}", why.answer);
        let mate = coach_answer(&req("best move?", "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1", &[]));
        assert!(mate.answer.contains("Ra8#"), "{}", mate.answer);
        let bad = coach_answer(&req("hello", "nonsense", &[]));
        assert!(!bad.answer.is_empty());
    }

    #[test]
    fn coach_explains_last_move() {
        let r = ChatRequest {
            question: "Why was my last move a mistake?".into(),
            // After 1.e4 e5 2.Qh5 Nc6 3.Bc4 Nf6?? (allows Qxf7#)
            fen: "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4".into(),
            moves_san: ["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6"].iter().map(|s| s.to_string()).collect(),
            engine_lines: vec!["M1: Qxf7#".into()],
            history: vec![],
        };
        let a = coach_answer(&r).answer;
        assert!(a.contains("Qxf7#"), "{a}");
    }

    #[tokio::test]
    async fn empty_question() {
        let m = Mentor::new(None, None);
        let r = m.chat(ChatRequest::default()).await;
        assert!(r.answer.contains(MENTOR_NAME));
    }

    #[test]
    fn sanitize_bounds() {
        let r = sanitize(ChatRequest { question: "é".repeat(5000), history: vec![ChatTurn::default(); 50], ..Default::default() });
        assert!(r.question.len() <= 4000);
        assert_eq!(r.history.len(), 20);
    }
}
