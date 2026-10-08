//! gm-mentor: the GrandMentor coach, "Mentor Mira".
//!
//! * [`explain_move`] — instant, rule-based, beginner-friendly explanation of a move
//!   (hanging pieces, forks, pins, skewers, mates, development, castling, center, trades...).
//! * [`describe_position`] — 3-6 short ideas/plans for a position.
//! * [`Mentor::chat`] — answers questions about a position. Uses the Anthropic Messages API when
//!   `ANTHROPIC_API_KEY` is set (model `GM_MENTOR_MODEL`, default `claude-opus-5-5`), and falls
//!   back to the rule-based coach on any failure or when no key is configured.
//!
//! Every function takes a [`Lang`]: rule-based text is written in that language (each one uses
//! its glossary in `docs/I18N.md` and per-piece gender/case agreement), and the LLM is told to
//! answer in it.
//!
//! All functions are panic-free on arbitrary input and allocate only bounded amounts of memory.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

pub use gm_content::Lang;
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

/// Rule-based, instant, friendly, 1-3 sentences, in `lang`.
pub fn explain_move(ctx: &MoveContext, lang: Lang) -> String {
    explain::explain_move(ctx, lang)
}

/// Plans / features of a position: material, king safety, open files, hanging pieces...
pub fn describe_position(fen: &str, lang: Lang) -> Vec<String> {
    describe::describe_position(fen, lang)
}

/// The rule-based chat answer (no network), in `lang`. Understands questions in any
/// supported language. Exposed for tests and offline use.
pub fn coach_answer(req: &ChatRequest, lang: Lang) -> ChatResponse {
    ChatResponse { answer: coach::answer(req, lang), source: "coach".to_string() }
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

    /// Answers a question in `lang`; falls back to the rule-based coach when the LLM is
    /// unavailable.
    pub async fn chat(&self, req: ChatRequest, lang: Lang) -> ChatResponse {
        let req = sanitize(req);
        if req.question.trim().is_empty() {
            let answer = match lang {
                Lang::En => format!("Hi, I'm {MENTOR_NAME}! Ask me anything about this position — the plan, the best move, or why a move was good or bad."),
                Lang::Es => format!("¡Hola, soy {MENTOR_NAME}! Pregúntame lo que quieras sobre esta posición: el plan, la mejor jugada o por qué una jugada fue buena o mala."),
                Lang::Pt => format!("Oi, eu sou {MENTOR_NAME}! Pergunte o que quiser sobre esta posição: o plano, o melhor lance ou por que um lance foi bom ou ruim."),
                Lang::Fr => format!("Salut, je suis {MENTOR_NAME} ! Pose-moi toutes tes questions sur cette position : le plan, le meilleur coup ou pourquoi un coup était bon ou mauvais."),
                Lang::De => format!("Hallo, ich bin {MENTOR_NAME}! Frag mich alles zu dieser Stellung – den Plan, den besten Zug oder warum ein Zug gut oder schlecht war."),
            };
            return ChatResponse { answer, source: "coach".into() };
        }
        if let Some(key) = &self.api_key {
            match self.try_llm(key, &req, lang).await {
                Ok(answer) => return ChatResponse { answer, source: "llm".into() },
                Err(e) => tracing::warn!(error = %e, model = %self.model, "mentor LLM call failed; using rule-based coach"),
            }
        }
        // The rule-based coach is CPU-only and fast (microseconds to a few ms).
        coach_answer(&req, lang)
    }

    async fn try_llm(&self, key: &str, req: &ChatRequest, lang: Lang) -> Result<String, llm::LlmError> {
        let _permit = tokio::time::timeout(Duration::from_secs(5), self.permits.acquire())
            .await
            .map_err(|_| llm::LlmError::Other("mentor is busy".into()))?
            .map_err(|_| llm::LlmError::Other("mentor shutting down".into()))?;
        match llm::call(&self.client, &self.endpoint, key, &self.model, req, true, lang).await {
            Err(llm::LlmError::BadRequest(msg)) => {
                tracing::debug!(%msg, "retrying mentor LLM call without optional parameters");
                llm::call(&self.client, &self.endpoint, key, &self.model, req, false, lang).await
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
        let r = m.chat(req("What should I do?", ITALIAN, &["+0.25: Bc5 c3 Nf6"]), Lang::En).await;
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
        let r = m.chat(req("plan?", ITALIAN, &[]), Lang::En).await;
        assert_eq!(r.source, "coach");
        assert!(!r.answer.is_empty());
    }

    #[test]
    fn coach_routes_keywords() {
        let plan = coach_answer(&req("What's the plan here?", ITALIAN, &["+0.25: Bc5 c3 Nf6"]), Lang::En);
        assert!(plan.answer.contains('•'), "{}", plan.answer);
        let eval = coach_answer(&req("Who is winning?", ITALIAN, &["+0.25: Bc5 c3 Nf6"]), Lang::En);
        assert!(eval.answer.contains("roughly equal"), "{}", eval.answer);
        // Hypothetical move: ...Nd4 is fine, but ...Qh4 just hangs nothing; ...Ba3 hangs the bishop.
        let why = coach_answer(&req("Why is Ba3 bad?", ITALIAN, &["+0.25: Bc5 c3 Nf6"]), Lang::En);
        assert!(why.answer.contains("bishop on a3"), "{}", why.answer);
        assert!(why.answer.contains("Bc5"), "{}", why.answer);
        let mate = coach_answer(&req("best move?", "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1", &[]), Lang::En);
        assert!(mate.answer.contains("Ra8#"), "{}", mate.answer);
        let bad = coach_answer(&req("hello", "nonsense", &[]), Lang::En);
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
        let a = coach_answer(&r, Lang::En).answer;
        assert!(a.contains("Qxf7#"), "{a}");
    }

    #[tokio::test]
    async fn empty_question() {
        let m = Mentor::new(None, None);
        let r = m.chat(ChatRequest::default(), Lang::En).await;
        assert!(r.answer.contains(MENTOR_NAME));
        let r = m.chat(ChatRequest::default(), Lang::Es).await;
        assert!(r.answer.starts_with("¡Hola, soy Mentor Mira!"), "{}", r.answer);
        for (lang, start) in [(Lang::Pt, "Oi, eu sou Mentor Mira!"), (Lang::Fr, "Salut, je suis Mentor Mira !"), (Lang::De, "Hallo, ich bin Mentor Mira!")] {
            let r = m.chat(ChatRequest::default(), lang).await;
            assert!(r.answer.starts_with(start), "{}", r.answer);
        }
    }

    #[test]
    fn coach_understands_spanish() {
        let lines = &["+0.25: Bc5 c3 Nf6"];
        // "What should I do?" -> best move.
        let a = coach_answer(&req("¿Qué debo hacer?", ITALIAN, lines), Lang::Es).answer;
        assert!(a.contains("**Bc5**") && (a.contains("motor") || a.contains("jugaría") || a.contains("más fuerte")), "{a}");
        let a = coach_answer(&req("mejor jugada", ITALIAN, lines), Lang::Es).answer;
        assert!(a.contains("**Bc5**"), "{a}");
        // Plan -> bullets with an evaluation in Spanish.
        let a = coach_answer(&req("¿Cuál es el plan?", ITALIAN, lines), Lang::Es).answer;
        assert!(a.contains('•') && a.contains("igualada"), "{a}");
        let a = coach_answer(&req("Explica la posición", ITALIAN, lines), Lang::Es).answer;
        assert!(a.contains('•'), "{a}");
        // Who is winning?
        let a = coach_answer(&req("¿Quién va ganando?", ITALIAN, lines), Lang::Es).answer;
        assert!(a.contains("más o menos igualada"), "{a}");
        // Why is a named move bad (English SAN and Spanish piece letters both work).
        let a = coach_answer(&req("¿Por qué es mala Ba3?", ITALIAN, lines), Lang::Es).answer;
        assert!(a.contains("alfil en a3") && a.contains("Bc5"), "{a}");
        let a = coach_answer(&req("¿Por qué es mala Aa3?", ITALIAN, lines), Lang::Es).answer;
        assert!(a.contains("alfil en a3"), "{a}");
        // "Why is it bad?" about the last move.
        let r = ChatRequest {
            question: "¿Por qué es malo?".into(),
            fen: "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4".into(),
            moves_san: ["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6"].iter().map(|s| s.to_string()).collect(),
            engine_lines: vec!["M1: Qxf7#".into()],
            history: vec![],
        };
        let a = coach_answer(&r, Lang::Es).answer;
        assert!(a.contains("Qxf7#") && a.contains("Ahora"), "{a}");
        // Threats, greetings, bad FEN.
        let a = coach_answer(&req("¿Hay alguna amenaza?", "6k1/5ppp/8/8/8/8/5PPP/R5K1 b - - 0 1", &[]), Lang::Es).answer;
        assert!(a.contains("Ra8#"), "{a}");
        let a = coach_answer(&req("hola", ITALIAN, &[]), Lang::Es).answer;
        assert!(a.contains("Mentor Mira") || a.contains("Encantada"), "{a}");
        let a = coach_answer(&req("hola", "nonsense", &[]), Lang::Es).answer;
        assert!(a.starts_with("No he podido leer"), "{a}");
        // English questions still route when the answer language is Spanish.
        let a = coach_answer(&req("What should I do?", ITALIAN, lines), Lang::Es).answer;
        assert!(a.contains("**Bc5**") && !a.contains("engine"), "{a}");
    }

    /// The same questions as `coach_understands_spanish`, asked and answered in Portuguese,
    /// French and German.
    #[test]
    fn coach_understands_new_languages() {
        struct Case {
            lang: Lang,
            best: [&'static str; 2],
            plan: &'static str,
            explain: &'static str,
            winning: &'static str,
            equal: &'static str,
            why_bad: [&'static str; 2],
            bishop: &'static str,
            last: &'static str,
            now: &'static str,
            threat: &'static str,
            hello: [&'static str; 2],
            bad_fen: &'static str,
        }
        let cases = [
            Case {
                lang: Lang::Pt,
                best: ["O que devo fazer?", "melhor lance"],
                plan: "Qual é o plano?",
                explain: "Explique a posição",
                winning: "Quem está ganhando?",
                equal: "mais ou menos igualada",
                why_bad: ["Por que Ba3 é ruim?", "Por que Ba3 é um lance ruim?"],
                bishop: "bispo em a3",
                last: "Por que esse lance foi ruim?",
                now: "Agora",
                threat: "Há alguma ameaça?",
                hello: ["olá", "oi"],
                bad_fen: "Não consegui ler",
            },
            Case {
                lang: Lang::Fr,
                best: ["Que dois-je faire ?", "meilleur coup"],
                plan: "Quel est le plan ?",
                explain: "Explique la position",
                winning: "Qui gagne ?",
                equal: "à peu près égale",
                why_bad: ["Pourquoi Ba3 est mauvais ?", "Pourquoi Fa3 est mauvais ?"],
                bishop: "fou en a3",
                last: "Pourquoi ce coup est mauvais ?",
                now: "Maintenant",
                threat: "Y a-t-il une menace ?",
                hello: ["bonjour", "salut !"],
                bad_fen: "Je n'ai pas pu lire",
            },
            Case {
                lang: Lang::De,
                best: ["Was soll ich tun?", "bester Zug"],
                plan: "Was ist der Plan?",
                explain: "Erkläre die Stellung",
                winning: "Wer steht besser?",
                equal: "ungefähr ausgeglichen",
                why_bad: ["Warum ist Ba3 schlecht?", "Warum ist La3 schlecht?"],
                bishop: "Läufer auf a3",
                last: "Warum ist dieser Zug schlecht?",
                now: "Jetzt",
                threat: "Gibt es eine Drohung?",
                hello: ["hallo", "Moin!"],
                bad_fen: "Ich konnte die aktuelle Stellung nicht lesen",
            },
        ];
        let lines = &["+0.25: Bc5 c3 Nf6"];
        for c in cases {
            let lang = c.lang;
            for q in c.best {
                let a = coach_answer(&req(q, ITALIAN, lines), lang).answer;
                assert!(a.contains("**Bc5**") && !a.contains('•'), "{lang} {q}: {a}");
                assert!(!a.contains("engine's") && !a.contains("I'd play"), "{lang} {q}: {a}");
            }
            for q in [c.plan, c.explain] {
                let a = coach_answer(&req(q, ITALIAN, lines), lang).answer;
                assert!(a.contains('•') && a.contains(c.equal), "{lang} {q}: {a}");
            }
            let a = coach_answer(&req(c.winning, ITALIAN, lines), lang).answer;
            assert!(a.contains(c.equal), "{lang}: {a}");
            for q in c.why_bad {
                let a = coach_answer(&req(q, ITALIAN, lines), lang).answer;
                assert!(a.contains(c.bishop) && a.contains("Bc5"), "{lang} {q}: {a}");
            }
            let r = ChatRequest {
                question: c.last.into(),
                fen: "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4".into(),
                moves_san: ["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6"].iter().map(|s| s.to_string()).collect(),
                engine_lines: vec!["M1: Qxf7#".into()],
                history: vec![],
            };
            let a = coach_answer(&r, lang).answer;
            assert!(a.contains("Qxf7#") && a.contains(c.now), "{lang}: {a}");
            let a = coach_answer(&req(c.threat, "6k1/5ppp/8/8/8/8/5PPP/R5K1 b - - 0 1", &[]), lang).answer;
            assert!(a.contains("Ra8#"), "{lang}: {a}");
            for q in c.hello {
                let a = coach_answer(&req(q, ITALIAN, &[]), lang).answer;
                assert!(a.contains("Mentor Mira") || a.contains("Nf3"), "{lang} {q}: {a}");
                assert!(!a.contains('•'), "{lang} {q}: greeting expected: {a}");
            }
            let a = coach_answer(&req("hallo", "nonsense", &[]), lang).answer;
            assert!(a.starts_with(c.bad_fen), "{lang}: {a}");
            // English questions still route; the answer stays in the requested language.
            let a = coach_answer(&req("What should I do?", ITALIAN, lines), lang).answer;
            assert!(a.contains("**Bc5**") && !a.contains("engine's"), "{lang}: {a}");
        }
    }

    /// Every language answers every kind of question with non-empty, fully filled text.
    #[tokio::test]
    async fn chat_never_empty_in_any_language() {
        let m = Mentor::new(None, None);
        let questions = ["", "hi", "best move", "plan", "who is winning", "threats?", "opening", "endgame", "Why is Ba3 bad?", "zzz"];
        let fens = [ITALIAN, "8/5k2/8/3P4/8/8/5K2/8 w - - 0 40", "R5k1/5ppp/8/8/8/8/8/6K1 b - - 1 1", "nonsense"];
        for lang in Lang::ALL {
            for fen in fens {
                for q in questions {
                    let r = m.chat(req(q, fen, &["+0.25: Bc5 c3 Nf6"]), lang).await;
                    assert!(!r.answer.trim().is_empty(), "{lang} {fen} {q}");
                    assert!(!r.answer.contains('{') && !r.answer.contains('}'), "{lang} {q}: {}", r.answer);
                }
            }
        }
    }

    #[test]
    fn sanitize_bounds() {
        let r = sanitize(ChatRequest { question: "é".repeat(5000), history: vec![ChatTurn::default(); 50], ..Default::default() });
        assert!(r.question.len() <= 4000);
        assert_eq!(r.history.len(), 20);
    }
}
