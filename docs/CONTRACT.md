# GrandMentor — Architecture & Integration Contract

GrandMentor is a local-first chess learning web app (think chess.com: play bots, eval bar,
game review with move classifications, a mentor/coach, lessons, openings, tactics puzzles,
endgame drills, saved games). **This document is the single source of truth that every
contributor codes against.** If you must deviate, keep the deviation inside your own files
and make it backward compatible with what is written here.

Goals: very fast, smart engine, zero leaks (bounded memory everywhere, every UI component
has `destroy()`), and a beautiful, beginner-friendly UI.

---

## 1. Repository layout & ownership

```
Cargo.toml                    workspace (members = crates/*)
crates/
  gm-engine/    chess engine: search + eval + engine pool         (shakmaty for move gen)
  gm-content/   loads & validates data/*.json; opening lookup     (types live here)
  gm-bots/      bot personalities, human-like move selection
  gm-analysis/  game review: move classification, accuracy, key moments
  gm-mentor/    coach explanations (rule-based) + optional Claude LLM chat
  gm-store/     SQLite persistence (rusqlite, bundled) + PGN import/export
  gm-server/    axum binary `grandmentor`: REST + WebSocket + static files
data/
  openings.json  puzzles.json  courses.json  endgames.json
web/                          static frontend, NO build step, native ES modules
  index.html
  css/          app.css (design system), board.css, pages.css ...
  js/app.js     router + shell
  js/api.js     REST client + EngineClient (websocket)
  js/settings.js  ui.js
  js/components/  board.js evalbar.js movelist.js sound.js evalgraph.js mentor.js clock.js
  js/pages/       home.js play.js analysis.js review.js puzzles.js learn.js lesson.js
                  openings.js endgames.js library.js profile.js settings.js
  vendor/chess.js  (chess.js 1.x ESM, BSD-2, vendored)
  img/pieces/<set>/{wK,wQ,wR,wB,wN,wP,bK,...}.svg
docs/  CONTRACT.md (this) FEATURES.md STYLEGUIDE.md
```

Only edit files you own. Build: `cargo build --release`; run: `cargo run --release -p gm-server`
→ http://localhost:8080 (env `GM_PORT`, `GM_DATA_DIR` default `./data`, `GM_DB` default
`./grandmentor.db`, `GM_WEB_DIR` default `./web`, `ANTHROPIC_API_KEY` optional,
`GM_MENTOR_MODEL` default `claude-opus-5-5`).

Conventions: Rust 2021 edition, `serde` for all JSON with `#[serde(rename_all = "snake_case")]`
on enums. No `unsafe`. No `unwrap()` on user input paths. Everything bounded (TT size, caches
are LRU/capped, request sizes limited). Squares/moves on the wire are **UCI** (`e2e4`, `e7e8q`);
human display uses SAN. FENs are full 6-field FENs.

---

## 2. Shared JSON shapes

**Score** (always from WHITE's point of view on the wire):
`{"cp": 35}` or `{"mate": 3}` (mate>0 = white mates in N, mate<0 = black mates in N).
Rust: `#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)] #[serde(rename_all="snake_case")] pub enum Score { Cp(i32), Mate(i32) }` defined in `gm-engine`.

**Classification** (rust enum in gm-analysis, snake_case strings):
`brilliant | great | best | excellent | good | book | inaccuracy | mistake | miss | blunder | forced`

---

## 3. Rust crate APIs

### gm-engine
```rust
pub enum Score { Cp(i32), Mate(i32) }               // white POV at API boundary
pub struct SearchLimits { pub depth: Option<u8>, pub movetime_ms: Option<u64>,
                          pub nodes: Option<u64>, pub multipv: usize }   // Default: depth None, multipv 1
pub struct PvLine { pub score: Score, pub moves: Vec<String> /*uci*/, pub san: Vec<String> }
pub struct SearchInfo { pub depth: u8, pub seldepth: u8, pub nodes: u64, pub nps: u64,
                        pub time_ms: u64, pub lines: Vec<PvLine> }       // lines sorted best-first
pub struct Engine;  // owns a fixed-size transposition table
impl Engine {
  pub fn new(tt_mb: usize) -> Self;
  pub fn new_game(&mut self);                         // clear TT/history
  pub fn search(&mut self, pos: &shakmaty::Chess, limits: &SearchLimits,
                stop: &std::sync::atomic::AtomicBool,
                on_info: &mut dyn FnMut(&SearchInfo)) -> SearchInfo;
}
pub fn evaluate(pos: &shakmaty::Chess) -> i32;         // static eval, side-to-move POV, cp
pub fn parse_fen(fen: &str) -> Result<shakmaty::Chess, String>;
pub fn to_fen(pos: &shakmaty::Chess) -> String;
pub fn uci_to_move(pos: &shakmaty::Chess, uci: &str) -> Result<shakmaty::Move, String>;
pub fn move_to_san(pos: &shakmaty::Chess, m: &shakmaty::Move) -> String;
pub fn move_to_uci(m: &shakmaty::Move) -> String;   // standard (e1g1 for castling)
pub struct EnginePool;  // N engines; Clone (Arc inside)
impl EnginePool {
  pub fn new(size: usize, tt_mb: usize) -> Self;
  /// Run blocking work with an exclusive engine on a blocking thread.
  pub async fn with_engine<R: Send + 'static>(&self, f: impl FnOnce(&mut Engine) -> R + Send + 'static) -> R;
  pub fn size(&self) -> usize;
}
```
Engine targets: PVS alpha-beta, iterative deepening, aspiration windows, TT, null-move,
LMR, killer/history, quiescence with SEE, check extensions, tapered PeSTO-style eval +
mobility/king-safety/pawn structure, mate scoring, draw detection (repetition, 50-move,
insufficient material). Must reach depth 12+ in ~1s from the start position on a laptop.

### gm-content
```rust
pub struct Opening { pub id: String, pub eco: String, pub name: String, pub family: String,
  pub moves: String /* SAN, space separated, no move numbers: "e4 e5 Nf3 Nc6 Bb5" */,
  pub side: String /* "white"|"black" = whose repertoire */, pub popularity: u8 /*1-10*/,
  pub level: String /* beginner|intermediate|advanced */,
  pub description: String, pub ideas: Vec<String>, pub traps: Vec<String>,
  pub uci: Vec<String> /* filled by loader */, pub fen: String /* filled by loader: final position */ }
pub struct Puzzle { pub id: String, pub fen: String,
  pub moves: Vec<String> /* UCI. moves[0] = opponent's move played BEFORE the user is on move
                            (lichess convention); user plays moves[1], moves[3], ... */,
  pub rating: u16, pub themes: Vec<String>, pub popularity: i16 }
pub struct Course { pub id: String, pub title: String,
  pub category: String /* basics|openings|middlegame|tactics|strategy|endgame */,
  pub level: String, pub description: String, pub icon: String /* emoji */,
  pub lessons: Vec<Lesson> }
pub struct Lesson { pub id: String, pub title: String, pub summary: String, pub steps: Vec<Step> }
pub struct Step { pub text: String /* markdown-lite: **bold**, *italic*, line breaks */,
  pub fen: Option<String>, pub orientation: Option<String>,
  pub arrows: Vec<Arrow>, pub highlights: Vec<String> /* squares */,
  pub task: Option<Task> }
pub struct Arrow { pub from: String, pub to: String, pub color: String /* green|red|blue|yellow */ }
pub struct Task { pub kind: String /* "moves" */, pub prompt: String,
  pub solution: Vec<String> /* UCI, alternating: user, reply, user, ... from step.fen */,
  pub hint: Option<String>, pub success: String }
pub struct EndgameDrill { pub id: String, pub title: String, pub category: String /* basic|pawn|rook|minor|queen */,
  pub level: String, pub fen: String, pub goal: String /* "win"|"draw" */,
  pub description: String, pub hint: String, pub technique: Vec<String> }
pub struct Content { pub openings: Vec<Opening>, pub puzzles: Vec<Puzzle>, pub courses: Vec<Course>, pub endgames: Vec<EndgameDrill>,
  pub lang: Lang /* language of this instance's text */ }
impl Content {
  pub fn load(dir: &std::path::Path) -> anyhow::Result<Content>; // validates + fills derived fields; skips (and logs) invalid entries;
                                                                 // also loads dir/i18n/<lang>/ overlays and precomputes every language view
  /// Text in `lang` (overlays applied, English fallback); its opening book reports localized names.
  pub fn localized(&self, lang: Lang) -> std::sync::Arc<Content>;
  pub fn opening_ref(&self, id: &str) -> Option<OpeningRef>;   // name in this instance's language (knows "starting-position")
  /// Deepest named opening matching the position (key = first 4 FEN fields) + book continuations.
  pub fn lookup_opening(&self, fen: &str) -> Option<OpeningMatch>;
  pub fn is_book_position(&self, fen: &str) -> bool;
}
pub struct OpeningMatch { pub opening: OpeningRef, pub continuations: Vec<BookMove> }
pub struct OpeningRef { pub id: String, pub eco: String, pub name: String }
pub struct BookMove { pub uci: String, pub san: String, pub name: Option<String> /* opening it leads to */, pub weight: u32 }

/// User-facing language, shared by every crate (serde: "en" | "es"; default En).
pub enum Lang { En, Es }
impl Lang {
  pub const ALL: [Lang; 5];                                     // En, Es, Pt (pt-BR), Fr, De
  pub fn code(self) -> &'static str;                            // "en" | "es" | "pt" | "fr" | "de"
  pub fn parse(tag: &str) -> Option<Lang>;                      // "es", "es-MX", "ES_es" -> Es; "pt-BR"/"pt-PT" -> Pt; "fr-CA" -> Fr; "de-AT"/"de-CH" -> De; unsupported -> None
  pub fn from_accept_language(h: &str) -> Option<Lang>;         // first supported range by q: "es-MX,es;q=0.9,en;q=0.8" -> Es
  pub fn negotiate(query: Option<&str>, accept_language: Option<&str>) -> Lang; // ?lang= > Accept-Language > En
}
pub mod words { /* per-language chess vocabulary + gender-aware template variables (PieceRef::vars, fill) */ }
```
Translation overlays: `data/i18n/<lang>/{courses,openings,endgames}*.json` (schema in `docs/I18N.md`),
deep-merged across files in name order. They replace text only (titles, descriptions, step text,
task prompt/hint/success, opening name/family/description/ideas/traps, endgame title/description/
hint/technique); moves, FENs, arrows, highlights and solutions always come from the English source;
missing ids/fields/steps fall back to English; unknown ids are ignored and counted in the load log.
Data files: `data/openings.json` = `[Opening]` (loader fills uci/fen), `data/puzzles.json` = `[Puzzle]`,
`data/courses.json` = `[Course]`, `data/endgames.json` = `[EndgameDrill]`. Serde: missing
`Vec`/`Option` fields default (`#[serde(default)]`). `cargo test -p gm-content` validates that
every move in every file is legal.

### gm-bots
```rust
pub struct BotProfile { pub id: String, pub name: String, pub elo: u16, pub avatar: String /*emoji*/,
  pub style: String /* e.g. "aggressive", "positional", "beginner", "trappy" */, pub description: String,
  pub greeting: String, pub category: String /* beginner|intermediate|advanced|master|coach */ }
pub struct BotMove { pub uci: String, pub san: String, pub chat: Option<String>, pub think_ms: u64 }
pub fn list(lang: Lang) -> Vec<BotProfile>;              // description/greeting in `lang`; id/name/style/category never change
pub fn get(bot_id: &str, lang: Lang) -> Option<BotProfile>;
pub fn choose_move(engine: &mut gm_engine::Engine, content: &gm_content::Content /* English source */, bot_id: &str,
                   start_fen: &str, moves: &[String], lang: Lang) -> Result<BotMove, String>; // chat in `lang`
```
~14 bots from ~250 to ~3000 Elo plus a "coach" bot. Weak bots blunder plausibly (not randomly):
limited depth/nodes, softmax over multipv with Elo-dependent temperature, occasional missed captures,
use opening book (content) with style preferences. `think_ms` = suggested UI delay for realism.

### gm-analysis
```rust
pub enum Classification { Brilliant, Great, Best, Excellent, Good, Book, Inaccuracy, Mistake, Miss, Blunder, Forced }
pub struct MoveReview { pub ply: usize /*1-based*/, pub san: String, pub uci: String, pub color: String,
  pub fen_before: String, pub fen_after: String, pub eval_before: Score, pub eval_after: Score,
  pub best_move_uci: String, pub best_move_san: String, pub best_line_san: Vec<String>,
  pub classification: Classification, pub win_chance_loss: f32, pub explanation: String,
  pub opening_name: Option<String>,
  pub reason: Option<reason::MoveReason> /* errors only; see "Why was that a mistake?" */ }
pub struct SideStats { pub accuracy: f32, pub estimated_elo: u16,
  pub counts: std::collections::BTreeMap<String, u32> /* classification -> count */ }
pub struct GameReview { pub start_fen: String, pub moves: Vec<MoveReview>,
  pub evals: Vec<Score> /* len = moves+1, evals[0] = start position */,
  pub white: SideStats, pub black: SideStats, pub opening: Option<gm_content::OpeningRef>,
  pub key_moments: Vec<usize> /* plies */, pub summary: String }
pub async fn review_game(pool: &gm_engine::EnginePool, content: std::sync::Arc<gm_content::Content>,
   start_fen: &str, moves: &[String], depth: u8,
   progress: Option<tokio::sync::mpsc::UnboundedSender<f32> /* 0..1 */>, lang: Lang) -> Result<GameReview, String>;
/// Same, with an external stop flag: raising `stop` halts every in-flight search and returns
/// `Err(REVIEW_CANCELLED)` promptly; dropping the future raises it too (no orphaned engine work).
pub async fn review_game_cancellable(/* same args */, stop: Arc<AtomicBool>) -> Result<GameReview, String>;
pub const REVIEW_CANCELLED: &str;
/// Rewrite only explanations, opening names and the summary in `lang` from the stored fields. No engine.
pub fn relocalize(review: &GameReview, content: &gm_content::Content, lang: Lang) -> GameReview;
pub fn to_stored_json(review: &GameReview, lang: Lang) -> Option<String>;   // review JSON + "lang" key (games.review_json)
pub fn from_stored_json(json: &str) -> Option<(GameReview, Lang)>;          // untagged legacy rows = En
pub fn win_percent(score: Score) -> f32; // lichess formula, white POV
```
Accuracy: lichess-style (win% → per-move accuracy, harmonic+volatility weighted mean).
Analyze positions in parallel across the pool. `explanation` filled via `gm_mentor::explain_move`.

### gm-mentor
```rust
pub struct MoveContext { pub fen_before: String, pub played_uci: String, pub played_san: String,
  pub best_uci: String, pub best_san: String, pub best_line_san: Vec<String>,
  pub eval_before: Score, pub eval_after: Score, pub classification: String }
pub fn explain_move(ctx: &MoveContext, lang: Lang) -> String;     // rule-based, instant, friendly, 1-3 sentences
pub fn describe_position(fen: &str, lang: Lang) -> Vec<String>;   // plans / features: material, king safety, open files, hanging pieces...
pub fn coach_answer(req: &ChatRequest, lang: Lang) -> ChatResponse; // rule-based; routes questions in every supported language
pub struct ChatRequest { pub question: String, pub fen: String, pub moves_san: Vec<String>,
  pub engine_lines: Vec<String> /* e.g. "+0.45: Nf3 Nc6 Bb5" */, pub history: Vec<ChatTurn> }
pub struct ChatTurn { pub role: String /* user|mentor */, pub text: String }
pub struct ChatResponse { pub answer: String, pub source: String /* "llm" | "coach" */ }
pub struct Mentor;  // holds reqwest client + optional API key
impl Mentor { pub fn from_env() -> Self; pub fn llm_enabled(&self) -> bool;
  pub async fn chat(&self, req: ChatRequest, lang: Lang) -> ChatResponse; } // falls back to rule-based coach; LLM told to answer in `lang`
```

### gm-store
```rust
pub struct Store;  // r2d2-free: a Mutex<rusqlite::Connection> (WAL) is fine; Clone via Arc
pub struct GameRecord { pub id: i64, pub white: String, pub black: String, pub result: String /* "1-0"|"0-1"|"1/2-1/2"|"*" */,
  pub termination: String, pub start_fen: String, pub moves: Vec<String> /*uci*/, pub pgn: String,
  pub bot_id: Option<String>, pub user_color: Option<String>, pub time_control: Option<String>,
  pub opening_name: Option<String>, pub accuracy_white: Option<f32>, pub accuracy_black: Option<f32>,
  pub review_json: Option<String>, pub notes: String, pub tags: Vec<String>, pub favorite: bool,
  pub created_at: String, pub updated_at: String }
pub struct GameSummary { /* all GameRecord fields except pgn, moves, review_json; plus move_count,
  final_fen: String /* full FEN after the last legal move; stored at save time, backfilled by migration v3
  and after a backup import */, last_move: String /* UCI, "" when no moves */ } 
pub struct NewGame { pub white, black, result, termination, start_fen, moves, bot_id, user_color,
  time_control, opening_name, notes, tags }   // pgn generated by store
impl Store {
  pub fn open(path: &Path) -> anyhow::Result<Store>;   // runs migrations
  pub fn list_games(&self, q: &GameQuery) -> anyhow::Result<Vec<GameSummary>>; // GameQuery{search, result, bot_id, favorite, limit, offset}
  pub fn get_game(&self, id: i64) -> anyhow::Result<Option<GameRecord>>;
  pub fn create_game(&self, g: &NewGame) -> anyhow::Result<GameRecord>;
  pub fn update_game(&self, id: i64, patch: &GamePatch) -> anyhow::Result<Option<GameRecord>>; // notes,tags,favorite,result,termination,moves,review_json,accuracy_*
  pub fn delete_game(&self, id: i64) -> anyhow::Result<bool>;
  pub fn import_pgn(&self, pgn: &str) -> anyhow::Result<Vec<GameRecord>>; // multi-game PGN
  pub fn get_profile(&self) -> anyhow::Result<Profile>;   // Profile{name, avatar, puzzle_rating, puzzle_rd, rush_best, puzzles_solved, puzzles_failed, streak_days, last_active, settings_json}
  pub fn update_profile(&self, p: &ProfilePatch) -> anyhow::Result<Profile>;
  pub fn record_puzzle_attempt(&self, puzzle_id: &str, puzzle_rating: u16, solved: bool, time_ms: u64) -> anyhow::Result<PuzzleResult>; // Glicko-ish update; PuzzleResult{rating, delta}
  pub fn record_rush(&self, score: u32) -> anyhow::Result<u32 /*best*/>;
  pub fn rush_bests(&self) -> anyhow::Result<BTreeMap<String /*mode*/, u32>>;            // puzzle_profile.rs
  pub fn record_rush_best(&self, mode: &str, score: u32) -> anyhow::Result<(u32 /*prev*/, u32 /*best*/)>;
  pub fn merge_rush_bests(&self, bests: &BTreeMap<String, u32>) -> anyhow::Result<BTreeMap<String, u32>>; // max per mode
  pub fn reset_puzzle_stats(&self) -> anyhow::Result<()>; // rating 1200 / RD 350, counters 0, attempts + puzzle rating history deleted
  pub fn set_lesson_progress(&self, course_id: &str, lesson_id: &str, completed: bool) -> anyhow::Result<()>;
  pub fn get_progress(&self) -> anyhow::Result<Vec<LessonProgress>>; // {course_id, lesson_id, completed, updated_at}
  pub fn stats(&self) -> anyhow::Result<Stats>; // see Stats below
}
pub mod pgn { pub fn to_pgn(...) -> String; pub fn parse_pgn(text: &str) -> Result<Vec<ParsedGame>, String>; }
pub struct Stats { pub games_played: u32 /* games with a user_color */, pub games_won: u32, pub games_lost: u32,
  pub games_drawn: u32, pub total_games: u32 /* every saved game, imports included */,
  pub win_rate: Option<f32> /* 0..100 over finished games */, pub per_bot: Vec<BotRecord> /* {bot_id, played, won, lost, drawn}, most played first */,
  pub avg_accuracy: Option<f32>, pub rating_history: Vec<RatingPoint> /* {at, rating}, oldest first, last 100 */,
  pub recent_results: Vec<RecentResult> /* {id, opponent, bot_id, user_color, result, outcome: "win"|"loss"|"draw"|"unfinished",
     accuracy, opening_name, created_at}, newest first, up to 10 */,
  pub puzzle_rating: f32, pub puzzle_rd: f32, pub puzzles_solved: u32, pub puzzles_failed: u32,
  pub rush_best: u32, pub streak_days: u32, pub lessons_completed: u32 }
```
Puzzle Rush bests per mode live in `rush_bests (id, mode, score, created_at)`: one row per new personal best, the
mode's best is `MAX(score)` (rows below it are pruned), so a backup **merge** keeps the best of both devices. Mode ids
match `[a-z0-9_-]{1,16}`, at most 16 modes.

---

## 4. HTTP API (gm-server, all JSON, prefix `/api`)

| Method & path | Request | Response |
|---|---|---|
| GET `/api/health` | – | `{ok, version, llm_enabled, engines}` |
| GET `/api/bots` | – | `[BotProfile]` |
| POST `/api/bot/move` | `{bot_id, start_fen, moves:[uci]}` | `BotMove` |
| POST `/api/engine/analyze` | `{fen, depth?, movetime_ms?, multipv?}` (movetime capped 10s) | `SearchInfo` |
| GET `/api/engine/ws` | WebSocket, see below | |
| POST `/api/review` | `{start_fen?, moves:[uci]}` or `{pgn}` or `{game_id}` (+`depth?` default 14) | `GameReview` (if game_id, cached into the game). The review stops as soon as the client disconnects (request future dropped) or the server shuts down (`503`), freeing the engines. |
| POST `/api/mentor/chat` | `ChatRequest` | `ChatResponse` |
| POST `/api/mentor/explain` | `{fen, move_uci}` | `{classification, explanation, best_move_san, best_line_san, eval_before, eval_after}` |
| GET `/api/mentor/position?fen=` | – | `{ideas:[string], eval: Score, best_line_san:[..]}` |
| GET `/api/games?search=&result=&bot_id=&favorite=&limit=&offset=` | – | `[GameSummary]` |
| POST `/api/games` | `NewGame` | `GameRecord` |
| GET/PUT/DELETE `/api/games/:id` | PUT: `GamePatch` | `GameRecord` / `{deleted:true}` |
| GET `/api/games/:id/pgn` | – | `text/plain` PGN (download) |
| POST `/api/games/import` | `{pgn}` | `[GameRecord]` |
| GET `/api/puzzles/next?theme=&min=&max=` | (defaults around user rating ±150) | `Puzzle` |
| GET `/api/puzzles/daily` | – | `Puzzle` (deterministic by date) |
| GET `/api/puzzles/themes` | – | `[{theme, count}]` |
| GET `/api/puzzles/rush?count=40` | – | `[Puzzle]` sorted by ascending rating |
| POST `/api/puzzles/:id/attempt` | `{solved, time_ms}` | `{rating, delta}` |
| POST `/api/puzzles/rush` | `{score, mode?}` (`mode` e.g. `"3"`, `"5"`, `"survival"`) | `{best}` + with `mode`: `{mode_best, previous_mode_best}` |
| GET `/api/puzzles/rush/bests` | – | `{bests: {mode: score}, overall}` (`overall` = best over all modes / `rush_best`) |
| POST `/api/puzzles/rush/bests` | `{bests: {mode: score}}` (max 16; migrates the browser's old local bests) | same as GET; the higher score wins per mode |
| GET `/api/courses` | – | `[{id,title,category,level,description,icon,lesson_count, lessons:[{id,title,summary}]}]` |
| GET `/api/courses/:id` | – | `Course` |
| GET `/api/progress` / POST `/api/progress` | POST `{course_id, lesson_id, completed}` | `[LessonProgress]` |
| GET `/api/openings?q=&side=&level=` | – | `[Opening]` (without long fields is ok) |
| GET `/api/openings/:id` | – | `Opening` |
| GET `/api/openings/lookup?fen=` | – | `OpeningMatch` or `null` |
| GET `/api/endgames` | – | `[EndgameDrill]` |
| GET/PUT `/api/profile` | PUT `ProfilePatch` | `Profile` |
| POST `/api/profile/puzzles/reset` | `{confirm: true}` (`400` otherwise) | `Profile` — puzzle rating back to 1200 (RD 350), solved/failed 0, attempts and puzzle rating history deleted; Rush scores, games and lessons kept |
| GET `/api/stats` | – | `Stats` |

Errors: HTTP 4xx/5xx with `{error: "message"}` (message in the request language, see below).

**Language.** Every endpoint accepts `?lang=en|es|pt|fr|de` (regional tags such as `pt-BR` map to their language); otherwise the `Accept-Language` header picks the
first supported language by q-weight (`es-MX,es;q=0.9,en;q=0.8` → `es`); otherwise English. Only
human text changes; JSON shapes, ids, enum values (`classification`, bot `style`/`category`,
puzzle themes), SAN/UCI/FEN and numbers never do. Localized: bot `description`/`greeting` and
move `chat`; mentor `explanation`, position `ideas` and rule-based chat (the LLM is told to answer in
the language; questions are understood in every supported language); review `summary`, per-move
`explanation`, `opening.name` and `opening_name`; course/lesson/step/task text, opening
`name`/`family`/`description`/`ideas`/`traps`, `BookMove.name` and the start position name in
`/openings/lookup`, endgame text (all from `data/i18n/<lang>/` overlays with English fallback;
`/openings?q=` matches both English and localized names); the opening name auto-detected by
`POST /games` / `/games/import`; and `{error}` messages of common failures (unknown messages pass
through in English). Reviews cached in `games.review_json` carry an extra `"lang"` key; `POST
/api/review {game_id}` in another language rewrites only the text from the stored evaluations
(the engine is not re-run) and caches that latest language; ad-hoc reviews do the same in memory.
The `/engine/ws` socket is language-neutral.

**WebSocket `/api/engine/ws`** (live eval bar / analysis lines). Client → server:
`{"type":"analyze","id":7,"fen":"...","multipv":3,"movetime_ms":4000,"depth":null}` (new analyze
cancels the previous one on this socket), `{"type":"stop"}`. Server → client:
`{"type":"info","id":7,"depth":12,"nodes":..., "nps":..., "lines":[{"score":{"cp":30},"moves":["e2e4",...],"san":["e4",...]}]}`
(throttled ≤ 10/s) and `{"type":"done","id":7, ...same fields}`. Server-side cap: movetime 30s.
Static files: everything else served from `web/` with gzip/br compression and caching headers;
unknown non-/api paths → `index.html`.

---

## 5. Frontend contract

No build step. Plain ES modules, modern browser. Dark theme by default (light theme toggle).
Hash routing: `#/`, `#/play`, `#/play/:botId`, `#/analysis?fen=..|?game=:id|?pgn=..`, `#/editor?fen=..`,
`#/review/:gameId`, `#/puzzles`, `#/puzzles/rush`, `#/puzzles/daily`, `#/learn`,
`#/learn/:courseId`, `#/learn/:courseId/:lessonId`, `#/openings`, `#/openings/:id`,
`#/endgames`, `#/endgames/:id`, `#/library`, `#/profile`, `#/settings`.

**Page module interface** (`web/js/pages/*.js`):
```js
export async function mount(root /* HTMLElement, already empty */, { params, query }) {
  // render into root; return a cleanup fn that removes ALL listeners/timers/sockets/components
  return () => { /* destroy */ };
}
export const title = 'Play';  // optional, used for document.title
```
The router (`app.js`) always calls the previous page's cleanup before mounting the next.

**`web/js/api.js`**
```js
export const api = { get(path), post(path, body), put(path, body), del(path) }; // path like '/api/games'; throws Error(message) on non-2xx
export class EngineClient {           // one websocket, lazy-connect, auto-reconnect
  analyze(fen, { multipv = 1, movetime_ms = 3000, depth = null } = {}, onInfo /* (info, done:boolean) */);
  stop(); close();
}
```
**`web/js/settings.js`**: `getSettings()`, `setSetting(key, value)`, `onSettingsChange(fn) → unsubscribe`.
Keys: `boardTheme` (green|brown|blue|purple|gray|contrast), `pieceSet` (cburnett|merida|alpha), `sounds` (bool),
`showCoords`, `showLegal`, `animationMs` (number), `showEvalBar`, `autoQueen`, `theme` (dark|light), `moveNotation` (san|figurine).
Accessibility keys (see "Accessibility" below): `highContrast` (bool), `cbPalette` (bool), `motion` (system|reduce|full),
`announceMoves` (bool, default true), `squareNames` (bool), `uiScale` (100|115|130).
Also exported: `reducedMotion()` (true when `motion` is reduce, or system + `prefers-reduced-motion`), `scrollBehavior()`
(`'auto'|'smooth'` for `scrollIntoView`), `UI_SCALES`.
**`web/js/ui.js`**: `toast(msg, kind)`, `modal({title, body /*Node|string*/, actions:[{label, kind, onClick}]}) → {close}`,
`h(tag, attrs, ...children)` tiny DOM helper, `icon(name)` → inline SVG string, `formatScore(score)` → "+1.2"/"M3",
`classificationMeta(cls)` → `{label, color, symbol /* "!!","!","★","👍","📖","?!","?","✗","??" */}`, `escapeHtml`.

**`web/js/components/board.js`**
```js
export class Board {
  constructor(el, { fen = 'start', orientation = 'white', interactive = true,
    movableColor = 'white' /* 'white'|'black'|'both'|null */, showCoords, showLegal, animationMs,
    onMove /* (move:{from,to,promotion,uci,san,fen /*after*/, captured, flags}) => boolean|void */,
    onSquareClick, sounds = true });
  setPosition(fen, { animate = true, lastMove = null /* [from,to] */ } = {});
  getFen(); setOrientation(color); flip(); get orientation();
  setInteractive(bool, movableColor);
  setArrows([{from, to, color}]); clearArrows();   // user right-drag arrows are separate & cleared on left click
  setHighlights([{square, kind /* 'hint'|'good'|'bad'|'selected'|'target' */}]); clearHighlights();
  setBadge(square, classification /* see ui.classificationMeta */); clearBadges();
  destroy();
}
```
Also in code (backwards compatible): constructor options `autoQueen` (undefined = follow the setting; `showCoords`,
`showLegal`, `animationMs` likewise fall back to user settings when undefined), `keyboard`, `announce`, `label`,
`premoveColor`, `onPremove`, `blindfold` (see Accessibility and Play experience below);
`setPosition(fen, {animate, lastMove, sound = true})` → `true` | `false` (invalid FEN ignored);
`move(uci | {from,to,promotion}, {animate = true, sound = true})` plays a move programmatically → move object | null;
`legalMoves()` (verbose chess.js moves); getter `chess` (the board's chess.js instance, read-only use);
`setCircles([{square, color}])`; `clearUserShapes()`; `getUserShapes()` → `{arrows, circles}` (right-click drawings,
cleared when the position changes).
Board validates legality with vendored chess.js (`/vendor/chess.js`), supports drag & click moves,
legal-move dots, promotion picker (or autoQueen), check highlight, last-move highlight, right-click
arrows/circles, smooth animations, sounds via `components/sound.js` (`playSound('move'|'capture'|'check'|'castle'|'promote'|'gameEnd'|'illegal'|'correct'|'wrong'|'notify')`, WebAudio synthesized and pre-rendered once into buffers, no files; `renderSound(name)` returns the rendered `AudioBuffer` for previews/tests). Move sounds play when the piece lands; a dragged piece lifts under the pointer on press, and captured pieces vanish when the attacker lands.
If `onMove` returns `false` the board reverts the move.

**`components/evalbar.js`**: `new EvalBar(el, {orientation})`, `.set(score, opts?)`, `.setOrientation(c)`, `.destroy()`.
`opts` resolves a finished mate (`{mate:0}`): `{mated:'white'|'black'}` or `{fen}` (side to move is mated); without it the bar keeps the side that was already ahead.
**`components/movelist.js`**: `new MoveList(el, {onSelect(ply), emptyText? /* default t('ui.movelist.empty') */,
showClassifications? /* default true */})`, `.setMoves([{san, classification?}], {startColor?: 'white'|'black',
startMoveNumber?} /* for games from a position */)`, `.setCurrent(ply /* 0 = start */)`, getters `.current`, `.length`,
`.destroy()`; keyboard ←/→ handled by the page, not the list. Appending moves re-renders only the new cells; SAN follows
the `moveNotation` setting live; at most 2000 moves are shown.
**`components/evalgraph.js`**: `new EvalGraph(el, {onSelect(ply)})`, `.setData(evals:[Score], classifications)`, `.setCurrent(ply)`, `.destroy()`.
**`components/mentor.js`**: `new MentorPanel(el, { getContext: () => ({fen, moves_san, engine_lines}) })`, `.say(text, {kind})`, `.destroy()` — a chat panel that calls `/api/mentor/chat`.
**`components/clock.js`**: `new ChessClock(el, {initialMs, incrementMs, onFlag(color)})`, `.start(color)`, `.press()`, `.pause()`, `.destroy()`.

CSS class vocabulary and tokens are defined in `docs/STYLEGUIDE.md` (written by the design-system owner).

## Accessibility

### Board keyboard & screen-reader support (`components/board.js`, backwards compatible)

```js
new Board(el, { …, keyboard /* default true: arrow keys etc. */, announce /* default true: speak moves */,
  label /* accessible name, default "Chess board" */ });
board.focus();                 // focus the keyboard cursor square
board.canUserMove(square?);    // interactive and the side to move (or the piece on `square`) is movable
board.playUserMove(uci | {from,to,promotion}); // play as if the user moved it (typed input): onMove decides; → move object | null
```
- The squares form an ARIA grid (`role="grid"` → 8 `role="row"` wrappers with `display: contents` → 64 `role="gridcell"`,
  in visual order) with one tab stop (roving `tabindex`). ←↑→↓ move the cursor (always screen directions, both
  orientations), Home/End jump within the row, PageUp/PageDown within the column, Enter/Space picks up a piece (legal-move
  dots appear) and drops it on a target (promotion opens the picker; arrows cycle its choices), Esc cancels a selection or
  a queued premove. Handled keys call `preventDefault()` + `stopPropagation()` so page shortcuts (← → navigation) don't
  also fire. Enter on a square also calls `onSquareClick(square)`.
- Cells are labelled `"e4, white knight"` / `"e4, empty"`, plus `selected`, `legal move`, `capture`, `in check`,
  `last move`, `premove` (blindfold hides piece names). The cursor ring (`.gm-kbd-cursor`) shows only after keyboard use.
- With `squareNames` on, a name tag shows on the hovered / focused square (`.gm-hover-name`, `.gm-sq-name`).
- Moves are announced (user moves, `move()`, and `setPosition(fen, {lastMove})` when it is exactly one legal move);
  after the opponent's move, "Your move" is added when the user can move. Preview boards pass `keyboard:false, announce:false`.

### `components/announcer.js`

```js
announce(text, { assertive = false, dedupe = true });  // shared polite/assertive live regions, 150 ms debounce,
                                                       // bursts joined, same text dropped within 1.2 s
clearAnnouncements();                                  // the router calls it on navigation
describeMove(move) → "White knight to f3" | "Black captures on d5, check" | "… Checkmate!"   // Board move or chess.js verbose move
announceMove(move); announceTurn(color, { you });      // respect the announceMoves setting
squareLabel(square, code?), pieceName(role), coloredPiece('wN')
```
`toast()` messages are spoken through `announce()` (errors/warnings assertive); the toast stack itself is not a live region.

### `components/moveinput.js`

```js
const mi = createMoveInput({ board /* Board or () => Board */, onMove? /* (mv) => false to reject */,
  blocked? /* () => message|null */, label?, placeholder?, className? });
panel.append(mi.el); …; mi.destroy();          // also mi.input, mi.focus(), mi.clear(), mi.setDisabled(bool)
parseMoveText(chessOrFen, text) → {from, to, promotion?, san} | null   // SAN (e4, Nf3, exd5, O-O, 0-0-0, e8=Q, e8Q, nf3) or UCI (e2e4, e7e8q)
looksLikeMove(text) → bool
```
Without `onMove` the move goes through `board.playUserMove()`, i.e. the page's normal `onMove` (puzzle checking, engine
replies…). Errors show under the field (`aria-invalid`) and are announced. Used on Puzzles (solver, mistakes, rush),
lessons, Analysis, Play a friend, endgame practice and the repertoire builder/drill; Play keeps its own typed-move box
(same parser). Quick drills have their own typed answers.

### `ui.js` additions
`tOr(key, englishFallback, params)` (for code that may run before i18n loads, e.g. the global error toast),
`focusableIn(container)`; modals trap Tab (also when nothing inside is focusable) and restore focus on close;
`classificationMeta(cls).color` follows the colour-blind palette when `cbPalette` is on (`cssVar` always follows the
active palette).

### `<html>` attributes set by `settings.js` (and the pre-paint script in `index.html`)
`data-contrast="high|normal"`, `data-cls-palette="cb|default"`, `data-motion="reduce|full"` (resolved, follows the OS while
`motion` is `system`), `data-square-names="on|off"`, `data-ui-scale="100|115|130"` (root `font-size` set inline).

## Local two-player (Play a friend)

Pass-and-play games between two people on one device. Frontend only; no new endpoints.

**Route:** `#/local` (page `web/js/pages/local.js`, styles `web/css/local.css` injected by the page, strings in the `local` locale namespace).
Optional query: `#/local?fen=<encodeURIComponent(FEN)>` preselects "Custom position" with that FEN (4-, 5- or 6-field FENs are accepted and normalized; invalid or already-finished positions show an inline error).

**Setup:** player names (empty = translated "White"/"Black"; White is prefilled with the profile name the first time), the same time-control presets as Play (`none`, `1+0`, `3+2`, `5+0`, `10+0`, `15+10`, `30+0`), start position, and options `autoFlip` (default on for `(max-width: 1024px), (pointer: coarse)`), `showLegal`, `evalBar` (default off; uses `EngineClient` over `/api/engine/ws`, toggleable in-game).
Preferences are stored in `localStorage["grandmentor.local.prefs.v1"]` = `{white, black, tc, autoFlip, showLegal, evalBar}`.

**Game:** game end is detected with the vendored chess.js (checkmate, stalemate, threefold repetition, 50-move rule, insufficient material) plus clock flags (`timeout`, or `timeout vs insufficient material` when the winner has only a king). Takeback undoes one ply after the opponent (the side to move) allows it; a draw offer from the side to move needs the other player's acceptance; resign asks which side resigns. The clock pauses while a request dialog is open.

**Saving:** when a game with at least 2 plies ends it is saved with `POST /api/games`:
`{white, black, result, termination, start_fen, moves, bot_id: null, user_color /* 'white'|'black' when exactly one name equals the profile name, else null */, time_control /* tc id or null */, opening_name: null /* server detects */, notes /* takeback count */, tags: ["local"]}`.
The server logs the `local_game` activity for `bot_id: null`. The game-over modal offers "Review this game" (→ `#/review/<id>`), a rematch with sides swapped, and a new game.

**Resume:** an unfinished game is kept in `localStorage["grandmentor.local.current.v1"]` =
`{v: 1, white, black, startFen, moves /* UCI */, tcId, opts, orientation, clocks: {white, black} | null, takebacks, updatedAt}`
(written after every move and on `pagehide` / hidden / unmount; removed when the game ends or is discarded). The setup screen shows a resume card for it.

## Position editor (board editor)

Frontend only; no new endpoints.

**Route:** `#/editor[?fen=<encodeURIComponent(FEN)>][&orientation=black]` (page `web/js/pages/editor.js`, nav item
`analysis`, strings in the `editor` locale namespace, styles `web/css/editor.css` injected by the component).
`fen` may have 1-6 fields; a readable but illegal position is loaded so it can be fixed, an unreadable one shows a
warning toast and the start position. While editing, the page keeps the URL in sync (`history.replaceState`, 400 ms
debounce) so a reload or a shared link reopens the same position.
Actions: **Analyse** → `#/analysis?fen=..[&orientation=black]`; **Play vs a bot** → `#/play?fen=..&color=w|b` (colour
picker, default = side to move); **Play with a friend** → `#/local?fen=..`; **Copy FEN**; **Share link** (copies
`<origin>/#/editor?fen=..`). Analyse/Play are `aria-disabled` while the position is invalid; the two Play actions also
while it is already checkmate, stalemate or insufficient material.
Entry point: the Analysis page's "Set up" action opens `#/editor?fen=<current position>` (it replaced the old setup
modal).

**Component** `web/js/components/editor.js`:
```js
const ed = new PositionEditor(el, { fen /* default start */, orientation /* 'white'|'black' */,
  onChange /* (fen, { valid, errors: string[] }) => void, called after every edit (and once on create) */ });
ed.getFen();            // full 6-field FEN; castling and en passant are sanitised against the placement
ed.setFen(fen) → bool;  // false (position unchanged) when unreadable
ed.valid; ed.errors;    // translated reasons; [] = playable
ed.gameOver;            // null | 'checkmate' | 'stalemate' | 'insufficient' (valid positions only)
ed.setTool('move' | 'erase' | 'wK' … 'bP'); ed.flip(); ed.setOrientation(color); ed.orientation;
ed.root; ed.sideEl;     // the editor's DOM; `sideEl` is an empty slot in the side column for page actions
ed.destroy();           // removes every listener, timer, ghost element and the Board
```
Also exported (pure helpers): `parseFen(raw) → {arr, turn, castling:Set, ep, half, full} | null`, `parsePlacement`,
`placementOf(arr)`, `possibleCastling(arr) → {K,Q,k,q}`, `epCandidates(arr, turn) → ['c6', …]`,
`positionErrors(state, fen) → string[]`, `START_FEN`.
- Built on `Board` (view-only: `interactive:false, sounds:false, announce:false`): the editor intercepts pointer and
  Enter/Space/Delete/Backspace/Escape keys in the capture phase on the board slot; Board still renders pieces, coordinates,
  check highlight and the ARIA grid with arrow-key navigation.
- Palette (white and black pieces, a move tool and an eraser; `role="toolbar"`, buttons with `aria-pressed`): click or
  Enter selects a tool, drag a piece straight onto the board. Board: click/tap applies the tool (same piece again removes
  it); pieces drag anywhere; dropping off the board, right-click and a 500 ms long-press remove a piece. Placing a king
  moves that side's existing king. Keyboard with the move tool: Enter picks a piece up, Enter on another square drops it.
  Every edit is announced (`editor.announce.*`).
- Controls: starting position, clear, flip, side to move, castling checkboxes (disabled unless king and rook stand on
  their home squares), en-passant select (only squares a pawn could really capture on), and a live FEN field (typing a
  readable FEN updates the board immediately; blur/Escape restores the canonical FEN).
- Validation (`editor.errors.*`): exactly one king per side, no pawns on the 1st/8th rank (squares listed), at most 8
  pawns and 16 pieces per side, the side not to move must not be in check, then chess.js `validateFen` + load.

## Installable app (PWA)

GrandMentor can be installed as an app and keeps puzzles and lessons working without a connection.

**Static endpoints (served by `crates/gm-server/src/web.rs`, not under `/api`)**

| Path | Notes |
|---|---|
| `GET /sw.js` | The service worker. `Cache-Control: no-cache`, `Service-Worker-Allowed: /`, `text/javascript`. The literal token `__GM_BUILD__` in `web/sw.js` is replaced by the build id (hash of every shell file's path + size + mtime, plus the crate version), so any frontend change ships a byte-different worker. 404 if the file is missing or > 512 KB. |
| `GET /precache-manifest.json` | `{"version": "<build id>", "files": ["/index.html", "/css/app.css", ...]}` — every file under `web/` except `dev/`, dot-files, `*.br`/`*.gz`, symlinks and `sw.js` itself. Bounded walk (depth 8, max 1500 files) on a blocking thread. `no-cache`. |
| `GET /manifest.webmanifest` | `application/manifest+json`, `no-cache`. Icons live in `web/img/icons/` (`icon-192/512.png`, `maskable-192/512.png`, `apple-touch-icon.png`, generated from `web/favicon.svg`). |

**Service worker (`web/sw.js`) — caches (all bounded; unknown `gm-*` caches are deleted on activate)**

| Cache | Strategy | Contents | Limit |
|---|---|---|---|
| `gm-shell-<build>` | precache, cache-first | the precache manifest list; navigations fall back to the cached `/index.html` | exact list |
| `gm-runtime-v1` | cache-first / SWR | same-origin static files missed by the precache; Google Fonts (SWR) | 120 entries |
| `gm-content-v1` | stale-while-revalidate | `GET /api/courses[/:id]`, `/api/openings[/:id]`, `/api/endgames[/:id]`, `/api/classics[/:id]`, `/api/puzzles/themes`, `/api/bots`, `/api/puzzles/:id`, plus the offline puzzle pack | 250 entries |
| `gm-api-v1` | network-first, cached fallback | any other JSON `GET /api/*` (profile, progress, stats, summaries…) | 80 entries |

- Cache keys for `/api` include the request language (`?__lang=<Accept-Language>`), since content is localized. Offline with nothing cached in the current language, the same content in another language is served.
- Eviction is LRU-ish: every write re-inserts the key at the end, the oldest keys are evicted beyond the limit.
- **Never cached:** non-GET requests, `/api/health`, `/api/engine/*` (incl. the WebSocket), `/api/mentor/*`, `/api/backup*`, anything containing `/export`, `*/pgn`.
- **Offline puzzles:** `GET /api/puzzles/next[?theme=]` and `GET /api/puzzles/rush[?count=]` are network-first; offline they are answered from a pack of up to 200 puzzles (`/api/puzzles/rush?count=200`), honouring `theme` when possible and avoiding recent repeats. Offline responses carry `X-GM-Offline: 1`.
- **Prefetch:** when the page is idle (`requestIdleCallback`, ≥ 4 s after load, skipped on Save-Data unless installed) `pwa.js` posts `{type:'prefetch', lang}`; the worker caches `/api/courses` + every course (lessons included, max 80 courses), the puzzle pack, themes, openings, endgames, profile and progress. Throttled to once per 12 h per language (IndexedDB `gm-pwa/meta`); forced after `appinstalled`; re-run for a new language after a language switch.
- **Offline write queue:** `POST /api/puzzles/:id/attempt`, `POST /api/progress`, `POST /api/puzzles/rush` and `POST /api/activity` that fail with a network error are stored in IndexedDB (`gm-pwa/queue`, max 200 entries, max 8 KB body, dropped after 30 days) and answered with `200` + `X-GM-Queued: 1` and a plausible body (`{rating, delta: 0, queued: true}` from the cached profile; the merged progress list — also written back into the cache so the course page shows the tick; `{best, queued: true}`; otherwise `{queued: true}`). Replay is in order, on Background Sync (`gm-replay`), on worker activation, and when the page reports it is back online. 2xx and 4xx remove an entry; network errors and 5xx stop the replay and keep the rest. Only these record-a-fact endpoints are queued; a write that reached the server but whose response was lost may be recorded twice.
- **Messages** page → worker: `{type:'skipWaiting'}`, `{type:'replay'}`, `{type:'prefetch', lang, force?}`, `{type:'status'}` (replies `{build, queued}` on `ports[0]`). Worker → page (`source: 'gm-sw'`): `queued`, `replayed {count, remaining}`, `prefetched {courses, lessons, puzzles}`.

**Frontend (`web/js/pwa.js`)** — `initPwa()` (called once from `app.js` boot after the shell renders) and `destroyPwa()`.
- Registers `/sw.js` (scope `/`, `updateViaCache: 'none'`) in secure contexts (https or localhost); checks for updates when the tab becomes visible (at most every 30 min).
- Update flow: a waiting worker shows a sticky toast "A new version is available — Reload"; Reload posts `skipWaiting` and reloads on `controllerchange` (fallback reload after 3 s).
- Offline: `navigator.onLine` + a `/api/health` probe (4 s timeout; re-probed every 15 s while offline and on `online`). Shows `#offline-banner` ("Offline — puzzles and lessons still work", Retry) and sets `html.is-offline`; "Back online" for 2.5 s when the connection returns, then asks the worker to replay the queue.
- Install: captures `beforeinstallprompt` and adds an "Install app" entry to the sidebar footer and the mobile "More" sheet (re-injected when `app.js` re-renders the shell); on iOS Safari (not standalone) the entry opens "Share → Add to Home Screen" instructions. Hidden once installed / in standalone mode.
- Keeps `<meta name="theme-color">` in sync with the in-app theme (`--bg-elev`).
- Strings: `pwa.*` in `web/locales/{en,es}/pwa.js`. Styles: `web/css/pwa.css`.

## Daily plan, streaks and goals

All days are **UTC calendar days** (`YYYY-MM-DD`, SQLite `date('now')`), used consistently by the activity log,
`profile.last_active` and every streak computation.

**Streak — single source of truth** (`gm_store::activity::streak_in`): active days = days with a non-zero count in
the `activity` table ∪ the run stored in `profile.last_active`/`profile.streak_days` (kept for streaks earned before
the activity log existed; store writes and `log_activity` refresh it with the computed run). `current` counts back
from today, or from yesterday when today has no activity yet (a streak only breaks after a whole missed day).
`Profile.streak_days`, `Stats.streak_days` and `/api/daily` all read this value.

Store (`gm_store::activity`): `log_activity(kind, n)`, `activity_days(days)`, `KINDS` (unchanged) plus
`streak() -> Streak`, `daily_goal() -> DailyGoal`, `set_daily_goal(&DailyGoal)`, `daily_summary() -> DailySummary`.
Goal table `daily_goal(id=1, kind, target)`; default `{kind:"minutes", target:10}`. Minutes are estimated per kind
(`activity::minutes_per`: game/local_game 10, classic 5, lesson 4, endgame 3, puzzle/drill 2, reviews 1).

| Method & path | Body | Response |
|---|---|---|
| GET `/api/daily` | – | `DailySummary` |
| PUT `/api/daily/goal` | `{kind:"minutes"\|"activities", target}` (minutes 1..=240, activities 1..=100; else 400) | `DailySummary` |
| GET `/api/activity?days=N` | N 1..=400 (default 84) | `{days, items:[{day, counts:{kind:n}, total}]}` (only active days, newest first) |
| POST `/api/activity` | `{kind, n?=1}` (kind ∈ `KINDS`, n 1..=20; else 400) | `DailySummary` — for client-only activities (endgame drills, reading classics…) |

```
DailySummary { date: "YYYY-MM-DD", goal: {kind, target},
  progress: {minutes, activities, value /* in goal unit */, target, ratio /* 0..1 */, met},
  streak: {current, best, today_active}, last7: [{day, total, active}] /* oldest → today */,
  today: {kind: count} }
```

**Frontend** — `web/js/components/daily.js`: `new DailyPanel(container)`, `.load({bots, courses, progress, games,
dailyPuzzle, nextLesson})`, `.destroy()`; also exports `choosePlan`, `taskDone`, `seededRandom`, `heatLevel`,
`ensureDailyCss()` (loads `web/css/daily.css`). Mounted on Home under the hero. The plan (3–5 tasks, ~10–15 min)
is composed client-side from `/api/puzzles/daily`, `/api/mistakes/summary`, `/api/repertoire/summary`, courses +
`/api/progress`, `/api/endgames`, `/api/adaptive/estimate` + `/api/bots` (missing endpoints are skipped), seeded by
the UTC date and cached per day in `localStorage['grandmentor.daily.plan.v1']` so it is stable across reloads.
A task is checked when today's count for its kind is > 0 (game task: `game` or `local_game`).
Pages that finish a client-only activity should call `api.post('/api/activity', {kind})` (endgames.js does for `endgame`).

## Learn from your mistakes

Spaced-repetition "find the better move" cards built from the user's own reviewed games.
Storage: `crates/gm-store/src/srs.rs` (tables `mistake_cards`, `mistake_scanned_games`); routes: `crates/gm-server/src/routes/mistakes.rs`.

**Ingestion.** When `POST /api/review` computes a review for a `game_id` and saves it, every move of the user's side
(`games.user_color`) classified `mistake` | `miss` | `blunder` with `win_chance_loss >= 10` and a different engine best
move becomes a card (max 12 per game, worst first). Cards are deduplicated by the first four FEN fields; removed cards
stay as tombstones and are never re-added. The deck is capped at 5000 live cards. Games without `user_color` are skipped.
`solution` is the best move plus the engine continuation while it stays forcing (the opponent has a single legal reply,
or the line is a forced mate), always odd length (ends on the user's move), max 9 plies.

**Scheduling** (Leitner / SM-2 hybrid): new cards are due immediately. A correct answer on a due card bumps `streak`
and grows the interval (1 day, then ≥ 3 days, then ≥ 7 days, × `ease`); at `streak` 3 the card graduates (retired).
A wrong answer (or hint / show solution) resets `streak` to 0, lowers `ease` (min 1.3) and brings the card back in 10 minutes.
Answering a card before it is due is practice: success leaves the schedule alone (`counted: false`), failure still resets it.
Each attempt logs activity `mistake_review`.

```
MistakeCard {
  id, fen /* user to move */, prev_fen?, prev_uci? /* opponent's previous move: prev_fen --prev_uci--> fen */,
  played_uci, played_san, best_uci, best_san, solution: [uci] /* starts with best_uci */,
  game_id?, ply /* 1-based */, move_number, color: "white"|"black", classification: "mistake"|"miss"|"blunder",
  phase: "opening"|"middlegame"|"endgame", opponent, bot_id?, explanation /* coach text of the played move */, lang,
  win_chance_loss, due_at /* ISO UTC */, due_in_secs /* <= 0 when due */, interval_days, ease, streak, reps, lapses,
  graduated, last_reviewed_at?, created_at
}
MistakeSummary { due, total /* live cards incl. graduated */, graduated, learning, next_due_at?, next_due_in_secs? }
```

| Method & path | Body / query | Response |
|---|---|---|
| `GET /api/mistakes/summary` | | `MistakeSummary` |
| `GET /api/mistakes/next` | `?exclude=<id>` (skip the card just answered) | `{ card: MistakeCard \| null, due: bool, summary }` — the most overdue card; if none is due, the soonest upcoming one (`due: false`). Graduated cards are never returned. `explanation` is in the request language. |
| `POST /api/mistakes/:id/attempt` | `{ solved: bool, time_ms?: n }` | `{ card, counted, graduated_now, summary }`; 404 if unknown/removed |
| `POST /api/mistakes/sync` | | `{ scanned, added, more, summary }` — backfill: scans up to 100 reviewed, not-yet-scanned games per call (`more: true` → call again) |
| `GET /api/mistakes` | `?filter=all\|due\|learning\|graduated&limit=1..100 (20)&offset=n` | `{ items: [MistakeCard], summary, limit, offset }` (newest first) |
| `DELETE /api/mistakes/:id` | | `{ ok: true, summary }`; 404 if unknown/removed |

**Frontend.** `#/puzzles/mistakes` (`params.mode === 'mistakes'` in `pages/puzzles.js`) runs the deck with the shared
`PuzzleRunner` (a card becomes `{ fen: prev_fen, moves: [prev_uci, ...solution] }`, or `{ fen, moves: solution, userFirst: true }`
when there is no previous move). On mount it calls `POST /api/mistakes/sync` (up to 3 rounds). The Puzzles hub shows a
"Learn from your mistakes" entry with the due-count badge (from `/api/mistakes/summary`). `#/puzzles?theme=<theme>` opens the
rated solver filtered by theme.

## Insights

Personal weakness tracker built from the user's reviewed games. Aggregation is pure and lives in
`gm_analysis::insights` (unit-tested); routes in `crates/gm-server/src/routes/insights.rs`.

**User side of a game:** `user_color`, else a case-insensitive match of the profile name against
`white`/`black`. Games where the side is unknown are ignored. "Reviewed" = the game has stored
accuracies (set by `POST /api/review {game_id}` or `review-next`).

### `GET /api/insights`
Computed server-side from at most 200 recent reviewed games (scan of the 500 most recent games),
cached in a bounded cache (8 entries) keyed by language + every reviewed game's `id:updated_at`
(any change invalidates). Text (`title`, `explanation`, `drill.label`) is in the request `Lang`.
```json
{
  "games_analyzed": 9, "min_games": 3, "ready": true,
  "total_games": 12, "unreviewed": 3, "reviewable": 3,
  "weaknesses": [{
    "id": "hanging_pieces|missed_tactics|endgame|opening|conversion|repeated|time_trouble",
    "title": "Leaving pieces unprotected", "explanation": "…", "count": 28, "games": 8, "score": 4.1,
    "theme": "fork",
    "examples": [{ "game_id": 7, "ply": 19, "fen": "<before the move>", "move_uci": "d1d3", "move_san": "Qd3",
                   "best_uci": "b5c4", "best_san": "bxc4", "color": "white", "classification": "blunder" }],
    "drill": { "href": "#/drills/hanging", "label": "Practice spotting hanging pieces" }
  }],
  "phases": [{ "phase": "opening|middlegame|endgame", "moves": 90, "inaccuracies": 15, "mistakes": 5, "blunders": 4 }],
  "hanging": [{ "piece": "pawn|knight|bishop|rook|queen", "count": 13 }],
  "tactics": [{ "theme": "hangingPiece", "count": 5 }],
  "accuracy_trend": [{ "game_id": 3, "date": "…", "accuracy": 72.6, "outcome": "win|loss|draw|ongoing", "color": "white" }],
  "average_accuracy": 73.8,
  "by_color": { "white": { "games": 6, "wins": 2, "losses": 4, "draws": 0, "accuracy": 73.0 } },
  "by_opening": [{ "name": "Italian Game", "games": 2, "wins": 1, "losses": 1, "draws": 0, "accuracy": 72.4 }],
  "conversion": { "winning_games": 3, "converted": 2,
                  "failed": [{ "game_id": 9, "ply": 41, "fen": "…", "color": "white", "outcome": "draw|loss", "best_cp": 1250 }] },
  "time_trouble": null
}
```
- `ready` = `games_analyzed >= min_games`; `reviewable` = unreviewed user games with ≥ 6 plies.
- `weaknesses`: at most 3, most urgent first; `theme` only for `missed_tactics`; `tactics` lists puzzle theme ids, most frequent first.
- `time_trouble`, when clock data exists: `{ games_with_clock, low_time_secs: 30, low_time_moves, low_time_errors, normal_moves, normal_errors }`.

Definitions: user errors = mistake, miss, blunder (phases also count inaccuracies). Phase: endgame when
non-pawn material of both sides ≤ 20 points (or no queens and ≤ 26), else opening while full move ≤ 10,
else middlegame. Hanging = a mistake/blunder after which a user piece can be won (SEE). Missed tactic =
the engine's best move on a user error is a fork / pin / skewer / discovered attack / back-rank mate /
mate in 1–3 / winning a hanging piece (`gm_mentor::tactics`). Conversion = finished game where the user
reached ≥ +3 and didn't win. Time trouble needs `[%clk]` comments in the stored PGN (currently stripped
on import, so it is normally `null` and the UI hides the card). Drill links: `#/drills/hanging`,
`#/puzzles?theme=<theme>`, `#/endgames`, `#/repertoire`, `#/play?fen=…&color=w|b`, `#/puzzles/mistakes`,
`#/puzzles/rush`.

### `POST /api/insights/review-next`
Body `{ "skip": [gameId…] }` (optional, ≤ 200 ids). Reviews ONE unreviewed user game (most recent
first, ≥ 6 plies) at depth 12 through the normal review pipeline and stores it like
`POST /api/review {game_id}`. → `{ "game_id": 12 | null, "remaining": 2 }` (`null` = nothing left).
Only one runs at a time (`409` otherwise). The review is awaited inside the request, so aborting the
HTTP request cancels its engine searches. The Insights page loops it for at most 10 games per click
and has a Stop button.

### Frontend
- `#/insights` (`web/js/pages/insights.js`, CSS `web/css/insights.css` linked from `index.html`).
  Exports `topWeaknessesCard() → { el, destroy() }` (used on the Profile page), `miniBoard(fen,
  { orientation, played, best })` (static SVG), `momentHref(gameId, ply)`.
- `#/review/:gameId?ply=<n>` opens Game Review on that move's walkthrough (added for Insights).
- Strings: `insights.*` in `web/locales/{en,es}/insights.js`.

## Backup & sync

Whole-profile backup file, restore (merge or replace) and LAN device sync. Store logic:
`crates/gm-store/src/backup.rs`; routes: `crates/gm-server/src/routes/backup.rs`; UI: the
"Your data" card on Settings (`web/js/components/backup.js`, `createBackupSection() -> { el, destroy }`).

### Backup file (format version 1)

```json
{ "format": "grandmentor-backup", "format_version": 1, "schema_version": 2,
  "app_version": "0.1.0", "created_at": "2026-10-08T12:00:00Z",
  "profile": { "name": "Alex", "...": "Profile fields" },
  "tables": { "games": [ { "id": 1, "white": "Me", "...": "..." } ], "activity": [ "..." ] },
  "browser": { "grandmentor.settings.v1": "{\"theme\":\"dark\"}", "gm.endgames.v1": "..." } }
```

* `tables` holds **every** user table (discovered with `pragma_table_list`; `sqlite_*` internals and
  `backup_meta` excluded), rows as JSON objects keyed by column name. Tables added by new features
  are included automatically. SQL values map to JSON null / integer / float / string; BLOBs to
  `{"$blob": "<hex>"}`.
* `browser` is added by the frontend: localStorage entries whose key starts with `grandmentor`,
  `gm.`, `gm_` or `gm-` (string values, at most 512 KB in total). The server never stores it; preview
  and import echo it back (sanitized) so the client can restore it.
* Max size 200 MB (`MAX_BACKUP_BYTES`); files with `format_version` > 1 are rejected.

### Endpoints

| Method & path | Body | Response |
|---|---|---|
| `GET /api/backup/status` | — | `{ last_backup_at, last_restore_at, last_sync_at, tables: [{name, rows}], total_rows, schema_version, format_version }` (timestamps ISO or null) |
| `GET /api/backup/export[?mark=false]` | — | the backup JSON (without `browser`), `Content-Disposition: attachment; filename="grandmentor-backup-YYYY-MM-DD.json"`. Records `last_backup_at` unless `mark=false`. |
| `POST /api/backup/preview` | backup file (raw JSON, ≤ 200 MB) | `{ format_version, schema_version, current_schema_version, app_version, created_at, profile_name, tables: [{name, rows, known, current_rows}], total_rows, warnings: [Warning], browser }` |
| `POST /api/backup/import?mode=merge\|replace[&confirm=replace][&source=sync]` | backup file | `ImportReport` = `{ mode, tables: [{name, inserted, updated, skipped, failed}], inserted, updated, skipped, failed, warnings: [Warning], browser }`. `mode=replace` requires `confirm=replace`; `source=sync` records `last_sync_at` instead of `last_restore_at`. |
| `GET /api/sync/pair` | — | `{ active, code?, expires_in?, network_visible }` — local requests only |
| `POST /api/sync/pair` | — | new pairing code (replaces any previous one): `{ active: true, code: "ABC-234", expires_in: 600, network_visible }` — local requests only |
| `DELETE /api/sync/pair` | — | revokes the code — local requests only |
| `GET /api/sync/snapshot` | header `X-GM-Pair: <code>` | backup JSON of this device (does not touch `last_backup_at`) |
| `POST /api/sync/merge` | header `X-GM-Pair`, backup file | `ImportReport` (merge mode; `browser` is always `{}`) |

`Warning` = `{ code, table?, column?, count }` with `code` one of `unknown_table` (skipped, `count`
rows), `unknown_column` (skipped), `rows_failed` (`count` rows violated a constraint and were
skipped), `newer_schema` (file from a newer DB schema) or `browser_dropped` (invalid browser entries
left out). The UI translates them. Errors are `{ "error": "..." }`, localized via `Accept-Language`:
400 bad / foreign file or unsupported version, 401 wrong or expired pairing code, 403 pairing
requested from another machine, 413 too large.

### Import semantics (always one transaction)

* **Replace**: empties every user table, inserts the backup rows verbatim (original ids, only
  columns known locally), drops rows with dangling foreign keys, keeps a profile row.
* **Merge** (idempotent — merging the same file twice changes nothing):
  * tables keyed by an auto-increment `INTEGER PRIMARY KEY` (games, puzzle_attempts, …): rows get
    new local ids and are de-duplicated by content — games by `(start_fen, moves, created_at)`,
    other tables by every column except the id and `updated_at`. Columns referencing those ids
    through a declared foreign key, or any column named `game_id`, are remapped to the new ids.
  * tables with a natural primary key (lesson_progress, activity, …): missing rows are inserted; an
    existing row is overwritten only when the backup's `updated_at` is newer.
  * `profile`: best-of counters (`rush_best`, `puzzles_solved`, `puzzles_failed`, at least the merged
    attempt counts), rating / RD / streak from the side with the newer `last_active`, local name /
    avatar / settings unless the local profile is still the default "Player".

### Device sync and security

Flow (device B, Settings → "Connect to another device"): B's browser fetches
`A/api/sync/snapshot` with the pairing code and posts it to its own
`/api/backup/import?mode=merge&source=sync` ("Bring here"), then posts its own
`/api/backup/export?mark=false` to `A/api/sync/merge` ("Send there"). "Both ways" pulls first.

* GrandMentor binds to `127.0.0.1` by default, so nothing is reachable from the network. To be a
  sync source, device A must be started with `GM_HOST=0.0.0.0` (or a LAN address), or have phone
  mode on (`GM_LAN=1` / Settings → Use on your phone), which also opens the plain HTTP port to the
  network. Everything else on A is then behind the access PIN (see "Phone & home use"); the two sync
  endpoints below are exempt from the PIN because they carry their own pairing code.
* Pairing codes (6 characters from a 32-letter alphabet, shown as `ABC-234`) live only in memory,
  one at a time, for 10 minutes; 20 wrong attempts revoke the code. They can only be created, read
  or revoked by requests from this machine (loopback peer address and, when present, a localhost
  `Origin`).
* `snapshot` / `merge` always require a valid `X-GM-Pair` header, with or without CORS.
* CORS for non-local origins is granted **only** to `/api/sync/snapshot` and `/api/sync/merge`,
  and only for requests carrying a currently valid code (preflights: when they announce the
  `x-gm-pair` header). `GM_SYNC_ALLOW_ORIGINS=http://192.168.1.21:8080,...` restricts it further to
  those origins. Every other endpoint keeps the localhost-only CORS policy.
* Traffic is plain HTTP (code and data are not encrypted): use it on your own network.
* The request body limit is raised to 200 MB only for `backup/preview`, `backup/import` and
  `sync/merge` (everything else stays at 1 MB); parsing and database work run on the blocking pool.

## Quick drills

Short, timed board-vision games at `#/drills` (hub) and `#/drills/:drillId` (`web/js/pages/drills.js`,
styles `web/css/drills.css`, strings `drills.*`). Scores are "higher is better"; the server keeps the personal
best per drill + variant (`drill_bests`) and a bounded history of 50 runs per drill + variant (`drill_scores`).

| Drill id | Variants (first = default) | Length | Questions |
|---|---|---|---|
| `coordinates` | `find-white`, `find-black`, `name-white`, `name-black` | 30 s | client-generated |
| `hanging` | `standard` | 60 s | server batch |
| `material` | `standard` | 60 s | server batch |
| `checks` | `checks`, `captures` | 60 s | server batch |
| `knight` | `basic`, `advanced` | 60 s | server batch |

**`GET /api/drills`** → `{ "drills": [ { "id": "coordinates", "variants": [ { "id": "find-white", "best": 23|null, "best_at": "ISO"|null, "plays": 4, "recent": [18, 20, 23], "last_accuracy": 95.0|null } ] } ] }`
(drills in the table order; `recent` = last ≤10 scores, oldest first).

**`GET /api/drills/:id/batch?n=20&variant=`** (`n` clamped to 1..=50; unknown drill → 404, unknown variant or
`coordinates` → 400) → `{ "drill": "hanging", "variant": "standard", "items": [...] }` where items are:
- `hanging`: `{ "fen", "hanging": [ { "square": "d5", "piece": "bN" } ] }` — 1..=3 pieces (either colour, never kings) that the
  other side can capture legally with a positive static exchange; positions where a side is in check are never served.
- `material`: `{ "fen", "white": 31, "black": 28, "diff": 3, "options": [3, -3, 2, 0] }` — pawns (P1 N3 B3 R5 Q9), `diff = white - black`, four distinct shuffled options.
- `checks`: `{ "fen", "answers": [ { "uci": "e7g7", "san": "Rg7+" } ] }` — every legal checking move (variant `checks`) or capture (`captures`), 1..=6 answers; the side to move is not in check and has no promotions.
- `knight`: `{ "fen", "start": "g1", "target": "f2", "min_moves": 2, "blocked": ["a1", ...], "path": ["e2", "f4"] }` — the FEN holds the white knight (and in `advanced` 2..=4 black pieces, no kings); `blocked` = squares occupied or attacked by black pieces; `path` = one shortest route (start excluded).

Positions come from `data/puzzles.json` (start and along the solution line), with random legal positions as a fallback.

**`POST /api/drills/:id/score`** `{ "variant": "find-white", "score": 14, "correct": 14, "total": 16, "duration_ms": 30000 }`
(`correct <= total <= 10000`, `score <= 10000`) → `{ "score": 14, "best": 14, "previous_best": 12|null, "is_best": true, "plays": 5 }`.
Logs activity kind `drill` (1 per run).

Store API (`gm_store::drills`): `DRILLS`, `is_known`, `variants_of`, `Store::record_drill(drill, variant, &DrillRun) -> DrillRecordResult`,
`Store::drill_stats() -> Vec<DrillStats>`. Generators live in `gm-server/src/routes/drills/generate.rs`.

Client preferences (mode / board side / coordinates / variant) are kept per viewer in `localStorage["gm.drills.v1"]`.
Keyboard: Enter/Space starts; type a square (`e4`) to click it; `1`–`4` pick an answer; in `checks` type a move (`g1f3`) and Enter for the next position.

## Interactive lessons

Extends `Task` (§3 gm-content) with new `kind`s. Module: `crates/gm-content/src/steps.rs`; the extra
fields live in `steps::TaskExtra`, flattened into `Task` on the wire and **omitted when empty**, so
`moves` tasks serialize exactly as before. `GET /api/courses/:id` returns them unchanged. An empty
`kind` means `moves`; an unknown kind drops the lesson at load time (and fails `cargo test -p gm-content`).

| `kind` | Learner does | Fields (besides `prompt`, `hint?`, `success`) | Load-time validation |
|---|---|---|---|
| `moves` | plays `solution` (replies auto-played) | `solution` | needs `fen`; every move legal |
| `guess` | guesses a master's moves one by one | `solution` (learner, reply, …), `notes: [string]` (one per learner move), `game?: string` caption | needs `fen`; moves legal; `notes.len() <= ceil(solution.len()/2)` |
| `count` | answers "who's ahead and by how much?" | `answer: i32` (pawns, White − Black, values 1/3/3/5/9), `choices: [i32]` (2–7, unique) | needs `fen`; `answer` equals the position's material balance and is one of `choices` |
| `hanging` | clicks every hanging piece, then **Check** | `squares: [sq]` | needs `fen`; each square holds a non-king piece that is attacked and undefended or attacked by a cheaper piece; no other piece of those colours qualifies |
| `choice` | picks the best option | `options: [{text, arrows: [Arrow], correct: bool, explain}]` (2–6, ≥ 1 correct) | squares/colours of arrows valid |
| `square` | clicks the named squares in order (coordinate quiz) | `squares: [sq]` (1–16), `blind?: bool` (hide board coordinates) | valid squares; `fen` optional (inherits the previous step's position) |

```jsonc
{ "kind": "count", "prompt": "Who is ahead, and by how much?", "answer": -2, "choices": [-3, -2, 0, 2, 3], "success": "…" }
{ "kind": "guess", "game": "Paul Morphy vs Duke Karl & Count Isouard, Paris 1858", "solution": ["c1g5", "b7b5", "c3b5", …], "notes": ["**Bg5** develops with a pin…", …], "prompt": "…", "success": "…" }
```

**Lesson player (`web/js/pages/lesson.js`).**
- `guess`: an exact guess scores **3** points (any mate also counts when the master's move mates). Otherwise the
  player compares the two resulting positions with `POST /api/engine/analyze` (`{fen, movetime_ms: 700, depth: 14}`,
  both in parallel, aborted when the step changes): loss ≤ 30 cp → **2**, ≤ 90 cp → **1**, else 0 (engine
  unavailable → 0, the lesson continues). Hints cap the move at 2 (piece shown) or 1 (arrow shown). The master's move
  is then animated with its note, the reply is auto-played, and the step total is shown; the finish card sums all
  guess steps of the lesson.
- `count`: answer buttons labelled "White +n / Equal / Black +n" (keys 1–9); a correct answer shows a per-side material breakdown.
- `hanging`: clicking a piece toggles it (only occupied squares); **Check** marks right picks green, wrong ones red, and says how many are still missing.
- `choice`: option arrows are drawn together and recoloured by position (blue, yellow, red, green), so the colour never gives the answer away; hovering/focusing an option shows only its arrows. Keys 1–9 pick an option.
- `square`: shows the target square big; `blind` hides `.gm-coord` labels via `.lrn-blind` on the board slot; reports the time taken.

**Translations.** `data/i18n/<lang>/courses*.json` task overlays accept, besides `prompt`/`hint`/`success`:
`game`, `notes: [string|null]` and `options: [{text?, explain?}|null]`, all in English order. Answers,
squares, arrows, `correct` flags and solutions always come from the English source. The Spanish text of the
**Board Vision** (`board-vision`) and **Guess the Move: Master Games** (`guess-the-move`) courses is in
`data/i18n/es/courses.part3.json`.

**Learn hub.** `#/learn` shows two practice entry cards between the stats and the course list: **Quick drills** → `#/drills`
and **Classic games** → `#/classics`.

## Play experience (adaptive bot, play from a position, premoves, confirm, blindfold)

### Adaptive bot & estimated rating

- **Bot** `id: "adaptive"` ("Sparky", `gm_bots::ADAPTIVE_ID`). `GET /api/bots` reports its `elo` as the user's
  current adaptive level. `POST /api/bot/move` with `bot_id: "adaptive"` plays at the stored level
  (`gm_bots::choose_move_at(engine, content, bot_id, start_fen, moves, elo: Option<u16>, lang)`; the level is
  clamped to `ADAPTIVE_MIN_ELO..=ADAPTIVE_MAX_ELO` = 250..=2800).
- `GET /api/adaptive/estimate` →
  `{"rating": n|null, "games": n, "provisional": bool, "bot_level": n, "bot_games": n}`.
  `rating` is null until the first counted game; `provisional` until 5 games.
- `POST /api/adaptive/result` `{"game_id": n}` → counts a **saved** game (`POST /api/games`) against a bot. Idempotent
  per game (a second call returns the stored change with `counted: false`). Response:
  `{"counted": bool, "previous": n|null, "rating": n, "delta": n, "games": n, "provisional": bool, "bot_level": n,
  "bot_level_delta": n, "estimate": {…estimate…}}`, or `{"skipped": "unfinished"|"not_bot"|"custom_position"|"too_short",
  "estimate": {…}}` when the game does not count (result `*`, no/unknown bot or user colour, non-standard start
  position, < 2 plies). Errors: 400 without `game_id`, 404 unknown game.
- Model (`gm_store::adaptive`): Elo update from prior 800 with K = 80 (first 5 games) / 48 (< 15) / 32, clamped
  100..3000, opponent = the bot's Elo (adaptive bot: its level at the time). Adaptive level: starts at the user's
  estimate (600 without one), moves ±160 → ±50 (shrinking per game played vs it) after a win/loss, unchanged on a draw.
  Store: `Store::adaptive_estimate()`, `Store::adaptive_record_game(game_id, opponent_elo, score, vs_adaptive)`.

### Play page URL

`#/play?fen=<encodeURIComponent(FEN)>[&color=w|b][&bot=<botId>]` opens the setup with that start position
(invalid or finished positions show a friendly toast and fall back to the normal start). `color` defaults to the side
to move. Games store it in `start_fen`. Entry points: Game Review toolbar (robot icon, current ply, same bot and the
user's colour) and the Analysis board ("Play bot" action, current node). `#/local` is linked from the setup.

Play prefs (`localStorage['grandmentor.play.prefs.v1']`) gain `extras: {premoves: true, confirmMove: false,
typeMoves: false, blindfold: false}`; saved unfinished games also keep `extras`.

### Board additions (`web/js/components/board.js`, backwards compatible)

```js
new Board(el, { …, premoveColor /* 'white'|'black'|null, default null */, onPremove /* (pm|null) => void */,
  blindfold /* bool, default false */ });
board.setPremoveColor(color|null);   // pieces of `color` can be queued while the other side is to move; null clears
board.getPremove();                  // {from, to, promotion?} | null
board.setPremove(pm|null);           // programmatic (e.g. a typed move); no onPremove call
board.clearPremove(notify = false);
board.playPremove();                 // plays it if legal now (calls onMove like a user move) → move object | null (dropped)
board.setBlindfold(bool); board.setPeek(bool); board.blindfold;   // hide pieces; peek shows them while held
```
Premove destinations are geometric (rays ignore blockers, pawn pushes/captures, castling squares while the rights
exist; never onto one's own piece). Promotion premoves open the picker (or queen with `autoQueen`). A queued premove
is drawn with `.gm-sq.premove` (tokens `--board-premove`, `--board-premove-light`, `--board-premove-dot`, with
fallbacks); right-click, clicking an empty/non-target square, or `clearPremove()` cancels it. With `premoveColor` set,
a premove selection or drag in progress survives `setPosition` / `setInteractive` (the opponent's move landing) and
turns into a normal move if it's now legal. Pages that don't pass the new options behave exactly as before.

## Classic games

Annotated library of famous public-domain games, narrated by the mentor (`#/classics`, `#/classics/:classicId`).

**Content** — `data/classics.json` (array), validated by `gm-content` (`classics.rs`; every move replayed at load,
`cargo test -p gm-content` fails if any game is dropped or the Spanish overlay is incomplete). Source fields:
`id, title, white, black, event, year, result ("1-0"|"0-1"|"1/2-1/2"), opening, level (beginner|intermediate|advanced),
themes [slug], orientation ("white"|"black"), summary, moves (SAN, space separated), key_ply,
annotations [{ply, text, label?, arrows?, highlights?}], questions [{ply, prompt, hint?, explanation, also?: [SAN]}]`.
- Annotation `ply` N is shown right after the N-th half-move (0 = intro). A `label` makes it a "key moment".
- Question `ply` N: the board stops after N−1 plies and the side to move must find the game's N-th move (or one of `also`).
- Theme slugs: development, attack, sacrifice, king-hunt, checkmate, tactics, positional, endgame, defence, opening-trap,
  back-rank, zugzwang, initiative, pawn-power, human-vs-machine, calculation (translated in `web/locales/*/classics.js`).
- Loader-filled: `uci [UCI]`, `san [SAN]`, `plies`, `key_fen` (FEN after `key_ply`), `era` (romantic ≤1885 | classical
  1886–1945 | modern 1946–1990 | computer 1991+), and per question `answer_uci`, `answer_san`, `accept [UCI]`.
- Spanish overlay `data/i18n/es/classics*.json`: `{ "<id>": { title, event, opening, summary, annotations: [{text, label}|null],
  questions: [{prompt, hint, explanation}|null] } }` (same order/length as English; text only).

**HTTP** (localized by `?lang=` / `Accept-Language`):
- `GET /api/classics` → `[ClassicSummary & {progress: ClassicProgress|null}]` where `ClassicSummary =
  {id, title, white, black, event, year, result, opening, level, themes, era, orientation, summary, key_fen, plies,
  annotation_count, question_count}`.
- `GET /api/classics/:id` → full `Classic` (all fields above) `& {progress: ClassicProgress|null}`; 404 if unknown.
- `POST /api/classics/:id/progress` body `{ply?: n, completed?: bool, answer?: {ply, correct}}` (at least one field;
  `ply` ≤ plies, `answer.ply` must be a question ply) → `ClassicProgress = {classic_id, last_ply, max_ply, completed,
  completed_at|null, answers: [{ply, correct}], updated_at}`. Only the first answer per question is kept; `completed`
  is never cleared. The first completion logs activity `classic`.

**Store** (`gm_store::classics`): `classic_progress_all()`, `classic_progress(id)`,
`update_classic_progress(id, &ClassicProgressUpdate) -> (ClassicProgress, newly_completed)`; tables `classic_progress`,
`classic_answers` (migration v2).

**Frontend** — `web/js/pages/classics.js` (+ `web/css/classics.css`, injected on mount; strings in `classics.*`).
Library: stats, level/era/theme/search filters, cards with a mini board of `key_fen`. Player: board + optional eval bar
(engine websocket, off by default), mentor bubble per annotation (arrows/highlights drawn), auto-play (slow/normal/fast,
longer pauses on comments) or step-by-step (←/→, Space, F), "pause and think" questions (move on the board; hint /
show me), key-moment chips, move list, "Play this position vs a bot" (`#/play?fen=…&color=w|b`) and "Open in Analysis"
(`#/analysis?pgn=…`). Speed and eval preference persist in localStorage `grandmentor.classics.v1`.

## Repertoire

Opening repertoire builder with spaced-repetition drills. Store: `crates/gm-store/src/repertoire.rs`
(`gm_store::repertoire::{Side, RepNode, RepTree, RepSummary, AddOutcome, Conflict, DrillLine, ReviewOutcome, Deviation, RepError}`);
routes: `crates/gm-server/src/routes/repertoire.rs`; page: `web/js/pages/repertoire.js` (`#/repertoire`, `#/repertoire/:side`,
query `?drill=1` starts the drill, `?node=<id>` selects a move); styles `web/css/repertoire.css` (injected by the page).

**Model.** One move tree per side (`white` | `black`), rooted at the standard initial position. A node is one move:
```jsonc
// RepNode
{ "id": 12, "parent_id": 0 /* 0 = first move */, "side": "white", "ply": 1 /* 1 = White's first move */,
  "uci": "e2e4", "san": "e4", "fen": "<full FEN after the move>", "note": "",
  "mine": true /* played by the repertoire's side = a drill card */,
  "ease": 2.5, "interval_days": 0, "reps": 0, "lapses": 0, "due": 1800000000 /* unix s */, "is_due": true,
  "last_review": null }
```
Rules: for your side at most **one** move per position (adding another returns a `conflict` unless `replace: true`,
which removes the old move and everything after it); opponent replies may branch freely. Bounds: ≤ 5000 nodes per
side, lines ≤ 80 plies, notes ≤ 500 chars. Every move is validated as legal. New cards are due immediately.
Scheduling (SM-2 style): correct on a due card → interval 1 d, 3 d, then × ease (ease +0.1, max 3.0, interval ≤ 365 d);
a correct answer on a card that is not due changes nothing; wrong → lapse, interval 0, ease −0.2 (min 1.3), due again in 10 min.

| Method & path | Body / query | Response |
|---|---|---|
| `GET /api/repertoire?side=white` | | `RepTree {side, nodes:[RepNode] (parents before children), stats:SideStats, max_nodes}` |
| `DELETE /api/repertoire?side=white` | | `{deleted:n}` |
| `GET /api/repertoire/summary` | | `{due, lines, new, cards, white:SideStats, black:SideStats}` — `SideStats = {nodes, cards, lines, due, new, learned, depth}` |
| `POST /api/repertoire/nodes` | `{side, parent_id?:0, uci, note?, replace?:false}` | `AddOutcome` |
| `POST /api/repertoire/lines` | `{side, parent_id?:0, moves:[uci…], replace?:false}` | `AddOutcome` |
| `PUT` (or `PATCH`) `/api/repertoire/nodes/:id` | `{note}` | `RepNode` |
| `DELETE /api/repertoire/nodes/:id` | | `{deleted:n}` (the move and everything after it) |
| `GET /api/repertoire/drill/next` | `?side=white\|black` (omit = both) `&any=1` (practice cards that are not due) | `{line: DrillLine \| null}` |
| `POST /api/repertoire/drill/attempt` | `{node_id, uci}` | `ReviewOutcome {correct, expected_uci, expected_san, card:RepNode}`; logs activity `repertoire_review` |
| `GET /api/repertoire/deviations` | `?limit=8` (max 30) | `[Deviation]` |
| `GET /api/repertoire/starters` | | `[{id, side, openings:[{id, name /* localized */}], lines}]` |
| `POST /api/repertoire/starters/:id` | | `{side, added, lines, skipped}` (lines clashing with your moves are skipped) |

`AddOutcome = {added, removed, path:[node ids of the whole line], conflict: null | {ply, parent_id, existing_id, existing_uci, existing_san, new_uci, new_san}}`
— with a conflict nothing is saved. Errors (`{error}`, localized en/es): 400 bad side / illegal move / line too long /
repertoire full / wrong side / drilling an opponent move; 404 unknown node or starter.

`DrillLine = {side, nodes:[RepNode] (path from the first move; always ends on one of your moves), due_in_line, due_total}`.
The server walks the tree preferring due, overdue and weak (low ease, lapsed) cards; opponent replies are picked at
random weighted by how much work their branch needs. The page's client-side "bot" plays the opponent moves of the line;
a wrong answer is recorded once, the right move is shown with a green arrow and the line is re-asked at the end.

`Deviation` (user games with `user_color`, standard start, newest first; games of a side with an empty repertoire are skipped):
`{game_id, side, white, black, result, created_at, opening_name, status, ply, played_uci, played_san, expected:[{uci,san}],
parent_id /* node whose position is fen_before; 0 = start */, fen_before, moves_before:[uci], book_plies}` with
`status` = `deviated` (you played a different move than your repertoire), `unprepared` (opponent move you have not
prepared — add it with `POST /nodes {side, parent_id, uci: played_uci}`), `end` (the game went past the end of your
preparation) or `followed` (the game ended inside the repertoire). Positions are matched by FEN, so transpositions count.

Shared helper for other pages: `import('./repertoire.js').then(m => m.openAddToRepertoire({ucis, sans?, name?, side?}))`
opens the "Add to my repertoire" dialog (used by the Openings detail page).

## Endgame training

Endgame theory drills with a short lesson and a "practise until reliable" mode against the engine.

**Content** (`data/endgames.json`, Spanish text in `data/i18n/es/endgames.json`). `EndgameDrill` gains:

```rust
pub struct EndgameDrill { /* id, title, category, level, fen, goal, description, hint, technique, plus: */
  pub variants: Vec<String>,   // extra start FENs (same side to move); practice picks one at random
  pub lesson: Vec<Step>,       // 2–5 "key idea" steps: text (markdown-lite), optional fen (default: drill fen),
                               // arrows [{from,to,color}], highlights [square]; no tasks
  pub success: DrillSuccess,   // when an attempt counts as solved
  pub pitfall: String }        // what usually goes wrong, shown after a failed attempt
pub struct DrillSuccess { pub kind: String /* mate|promote|bare_king|hold */, pub moves: u32 }
```

- `mate`: checkmate within `moves` own moves. `promote`: promote a pawn that the opponent cannot capture at
  once (or leave the opponent a bare king). `bare_king`: win all enemy material (or mate). These three go
  with `goal: "win"`; `hold` (only with `goal: "draw"`) succeeds on any draw by rule, when the engine
  has nothing left to win with (bare king or a lone minor piece), or after surviving `moves` own moves.
- Missing `success` defaults to `mate`/50 (win) or `hold`/30 (draw). The loader rejects bad goals/kinds,
  variants with the other side to move, `moves > 100`, more than 8 variants and lesson squares off the board.
- Categories (UI groups): `basic` Basic mates, `pawn` Pawn endgames, `rook` Rook endgames, `queen` Queen
  endgames, `minor` Minor pieces.

**Verification.** All 90 positions with ≤ 7 pieces (every main FEN and variant except three 8–10 piece pawn
structures) were checked against the Lichess Syzygy tablebase (`tablebase.lichess.ovh`): every `win` drill is
a tablebase win for the side to move and every `draw` drill a tablebase draw. The remaining six positions
(`outside-passed-pawn`, `pawn-breakthrough`, `rook-behind-passer` and their mirrors) are covered by the engine
test `crates/gm-server/tests/training.rs::drill_positions_pass_engine_sanity_check`, which also runs on every
position: wins must score ≥ +150 cp or a mate for the side to move, draws must not be a forced mate against it.

**HTTP**

| Method & path | Body | Response |
|---|---|---|
| GET `/api/training` | – | `{ "mastery_streak": 3, "drills": [DrillProgress] }` — one entry per drill in the content (zeros when never tried) |
| POST `/api/training/:drillId/attempt` | `{ "success": bool, "moves": n /* 0..=500, default 0 */ }` | `DrillProgress` + `"just_mastered": bool, "mastery_streak": 3` |

```json
DrillProgress = { "drill_id": "lucena", "attempts": 5, "successes": 4, "streak": 3, "best_streak": 3,
  "best_moves": 9, "mastered": true, "mastered_at": "2026-10-08T21:13:46Z",
  "last_at": "2026-10-08T21:14:02Z", "last_success": true }
```

Unknown drill → 404, `moves > 500` or a malformed body → 400 (`{"error": "..."}`). A drill is mastered after
3 successes in a row; mastery is sticky (a later failure only resets `streak`). Each attempt logs activity
kind `endgame`. Storage: `gm_store::training` (`Store::training_progress`, `Store::drill_progress`,
`Store::record_training_attempt`), table `endgame_training`.

**Frontend** (`web/js/pages/endgames.js`, styles in `web/css/endgames.css` + the existing `eg-*` rules in
`learn.css`, strings in the `endgames` locale namespace).

- `#/endgames`: drills grouped by category with mastered/total per group, progress dots (●●○) per drill and a
  gold medal on mastered drills.
- `#/endgames/:id`: the lesson (step through with Back/Next or ←/→) opens first until the drill has been
  attempted; then practice opens directly (the book button reviews the lesson). Practice plays the engine
  (`POST /api/engine/analyze`, 500 ms; falls back to the strongest bot) from the main FEN on the first attempt,
  then from a random variant. An attempt fails on checkmate, a draw by rule in a win drill, the move limit,
  the engine's evaluation turning lost (≤ −500 cp, or 600 cp worse than the start position for draw drills,
  or a forced mate) or drawn (|cp| ≤ 25 for two replies in a win drill). Results are posted to
  `/api/training/:id/attempt`; attempts that used help (the best-move hint or a takeback) are not recorded.
  Results link to `#/play?fen=<FEN>&color=w|b` ("Play this position vs a bot") and `#/analysis?fen=`.

## Opening practice vs a bot

A normal bot game from the **standard start** with an opening's moves already on the board (no backend change: the
frontend reuses `GET /api/openings/:id`, `GET /api/openings/lookup` and `POST /api/games`).

**URL:** `#/play[/:botId]?opening=<openingId>[&line=<uci uci …>][&color=w|b]`
- `opening` — id from `data/openings.json`; gives the name and the main line (its `uci`). Without `line`, the whole
  main line is pre-played.
- `line` — UCI moves from the standard start, separated by spaces (`%20`), `+` or commas; max 40 plies / 400 chars,
  every move legal and the game not over. Usable alone (e.g. from the repertoire): the banner name then comes from
  `/api/openings/lookup` of the final position, else "your line".
- `color` — the user's side (`w|b`, `white|black` also accepted); defaults to the opening's `side`, else white.
- With practice params, `fen` is ignored. An unknown opening, illegal/finished line or overlong line shows a friendly
  toast (`practice.error.*`) and the normal setup screen.
- The setup screen shows a "Practising: <name>" card (numbered line, "Normal start" drops it), the preview board at
  the line's final position, the chosen colour, and preselects the adaptive bot (else the recommended bot) unless
  `:botId` is given. The colour picked for a practice game is not stored as the default colour preference.

**Game:** `start_fen` stays the standard start and the pre-played moves are the first `moves` of the game, so it is
saved, reviewed, named (opening book) and counted by `/api/adaptive/result` like any other game. Extra on save:
tag `opening-practice` and a note `practice.notes`. Pre-played plies cannot be taken back; the abort rule (< 2 plies
not saved), draw offers and the clock start count only plies played after them. The game panel shows a
"Practising: <name>" banner with a gentle note when the game leaves the opening's main line (user: names the book
move; bot: "left the book line") or when `/api/openings/lookup` stops recognising the position. Rematch keeps the
practice; the unfinished-game save (`grandmentor.play.current.v1`) gains `practice: {id, name, moves, book, color}`.

**Entry points:** opening detail footer "Practice vs a bot" (main line; in Explore the explored line when it leaves
the main line) and the Train-complete card "Now play it against a bot"; repertoire move details "Practice this line vs
a bot" (`?line=…&color=` for the path to the selected move). Helpers: `web/js/components/practice.js`
(`parseLine`, `numberedSans`, `practiceHref`, `MAX_LINE_PLIES`). Strings: `practice` locale namespace.

## Why was that a mistake? (grounded review reasons)

Every inaccuracy, mistake, miss and blunder in a review carries an engine-grounded `reason`
(`gm_analysis::reason`). It is derived from two lines the review already searched (bounded, stoppable,
no extra engine time): the best line from `fen_before` and the opponent's best line from `fen_after`
(the refutation). Both are replayed on a board to find the concrete cause. Other moves have no
`reason` key (also absent in reviews stored before this existed; `force: true` recomputes).

```rust
pub enum ReasonKind { AllowsMate, HangsPiece, AllowsFork, AllowsPin, AllowsSkewer, LosesMaterial,
  MissedMate, MissedFork, MissedMaterial, Positional }            // snake_case on the wire
pub struct MoveReason { pub kind: ReasonKind, pub text: String,
  pub refutation_uci: Vec<String>, pub refutation_san: Vec<String>, pub refutation_key: Option<usize>,
  pub better_uci: Vec<String>, pub better_san: Vec<String>, pub better_key: Option<usize>,
  pub mate_in: Option<u32>, pub lost: Vec<String>, pub won: Vec<String>, pub targets: Vec<String>,
  pub material: i32 }
pub fn derive(input: &ReasonInput) -> Option<MoveReason>;   // pure; None for non-errors
pub fn render(reason: &MoveReason, lang: Lang) -> String;    // the friendly text
```

```json
"reason": { "kind": "loses_material", "text": "After Kxf7, you lose your queen for a pawn. Bc4 was better.",
  "refutation_uci": ["e8f7","f1c4","f7e8","g1f3"], "refutation_san": ["Kxf7","Bc4+","Ke8","Nf3"], "refutation_key": 0,
  "better_uci": ["f1c4","g7g6","h5d1","g8f6","b1c3","a7a6"], "better_san": ["Bc4","g6","Qd1","Nf6","Nc3","a6"], "better_key": null,
  "mate_in": null, "lost": ["queen"], "won": ["pawn"], "targets": [], "material": -8 }
```

- `refutation_*` start from `fen_after` (opponent to move); `better_*` start from `fen_before` and begin with
  `best_move_uci`. Lines are ≤ 12 plies; `*_key` is the index of the move that matters (the capture of the
  lost piece, the mating move, the fork). `lost`/`won`/`targets` are role names (`pawn`…`king`), most valuable
  first, after cancelling equal trades in the inspected window; `material` is the mover's net change in
  1/3/3/5/9 points over the refutation (or the gain of the better line for `missed_*`).
- Precedence: allows mate → missed mate → loses material (hangs a piece when the very next reply takes it for
  nothing; fork / pin / skewer when that reply is one) → missed material (fork) → positional (the opponent's
  best answer, with `targets` = a piece that answer attacks).
- `text` is in the request `Lang`; `relocalize` re-renders it from the structured fields. For every kind except
  `positional`, `explanation` is the same text (it replaces the one-ply rule-based explanation, so e.g.
  `Qxf7+??` is explained as losing the queen); for `positional` the rule-based `explanation` is kept.

**Frontend** (`web/js/components/whyline.js`, styles `web/css/whyline.css`, strings in the `why` locale
namespace): `new WhyPanel(host, { move, ply0, board, onStart, onExit })`, `.stop(restore = true)`,
`.destroy()`; `hasReason(move)`. In the review walkthrough the box shows a kind tag, the reason text (when it
differs from `explanation`), **Show me** (steps through the refutation from `fen_after`, red arrow on the next
move, key move outlined) and **Better: <move>** (the better line from `fen_before`, green). While a line is
shown the board is read-only; ←/→/Home/End step, Space plays/pauses, Esc or **Back to game** returns to the
game position. `review.js` stops the panel whenever it navigates (`stopLine`).

## Guided first week

A 7-day path for new players (`#/start`). The plan is static (`gm_store::first_week::PLAN`, 7 days × 2–4 steps);
every step deep-links into an existing feature and has a completion `Rule` checked against real activity:

| Rule (`kind`) | Done when |
|---|---|
| `lesson` | `lesson_progress` row completed (any time) |
| `drill` | the quick drill has at least one run (`drill_bests.plays > 0`) |
| `endgame` | the endgame drill has a success (`endgame_training.successes > 0`) |
| `game` | a game vs one of the listed bots created on/after the day unlocked (coach: `coach`, `coach-leo`; beginner: `pawnny`, `lulu`, `benny`, `rosa`) |
| `review` | a game with `review_json` updated since the path started |
| `puzzles` | N solved `puzzle_attempts` on/after the day unlocked with one of the themes (day 6: 3 × `fork`/`pin`) |
| `goal` | a daily goal has been saved (`daily_goal` row) |

Any step on an unlocked day can also be marked done by hand. Day *n* unlocks on `started_on + (n−1)` (UTC days,
same clock as the activity log); unlocked days stay open (catch-up). The first time a step is seen done on an unlocked
day it is persisted (`first_week_steps`), so progress never goes backwards and `newly_*` fields fire exactly once.
Offered on Home (`eligible`) while not dismissed and either in progress, or not started with < 3 games and < 3
completed lessons. Tables (migration v3): `first_week(id=1, started_on, dismissed, completed_on, updated_at)`,
`first_week_steps(step_id PK, manual, updated_at)`.

| Method & path | Body | Response |
|---|---|---|
| GET `/api/first-week` | – | `FirstWeekState` (persists newly detected steps) |
| POST `/api/first-week/start` | – | `FirstWeekState`; starts today if not started (idempotent), clears `dismissed` |
| POST `/api/first-week/restart` | – | `FirstWeekState`; Day 1 = today, forgets all steps |
| POST `/api/first-week/dismiss` | `{dismissed?: bool = true}` | `FirstWeekState` |
| POST `/api/first-week/step` | `{step_id, done?: bool = true}` | `FirstWeekState`; 400 for unknown step, locked day or not started. `done:false` only removes manual marks |

```
FirstWeekState { started, started_on: "YYYY-MM-DD"|null, today, dismissed, eligible,
  unlocked_days, completed_days, week_complete, current_day: 1..7|null /* first open, unfinished day */,
  days: [{ day: 1..7, id, emoji, unlock_on /* "" when not started */, unlocked, done,
           steps: [{ id, kind, href /* hash route; review → #/review/<latest game> */, done, manual,
                     progress: {have, need}|null }] }],
  newly_done: [step_id], newly_completed_days: [n], week_just_completed }
```

**Frontend** — `web/js/pages/start.js` (`#/start`; opening it starts the path), `web/js/components/firstweek.js`:
`ensureFirstWeekCss()` (`web/css/firstweek.css`), `dotPath(state, {compact})`, `celebrate(state, {bag})` (toast +
confetti, confetti skipped under reduced motion), `new FirstWeekCard(container)` (Home card: `.load()`, `.destroy()`;
hides itself when not `eligible`; "I already know how to play" dismisses), `createFirstWeekSection()` (Settings card
with "Start over" / "Show on Home": `{el, destroy}`). Strings: `firstweek` locale namespace (step/day copy keyed by id).

## Weekly personal set

"Your weekly set": ~12 puzzles per ISO week (UTC) built from the user's weakest tactic themes. Storage:
`crates/gm-store/src/weekly.rs` (tables `weekly_sets`, `weekly_items`, created in schema migration v3); routes and
pure logic: `crates/gm-server/src/routes/weekly.rs` + `routes/weekly/plan.rs`.

**Ranking.** Every theme in `plan::TRACKED` (fork, pin, skewer, hangingPiece, discoveredAttack, backRankMate,
mateIn1/2/3, ... — only themes with ≥ 6 pack puzzles) gets a weakness score from the last 60 days, each observation
weighted by recency (half-life 21 days):
- rated puzzle attempts (`puzzle_attempts` joined with the pack's themes): fail rate per theme, smoothed towards the
  user's overall fail rate (4 pseudo-attempts), minus that overall rate, × 2;
- mistakes from reviewed games: every live mistake card (`mistake_cards`) is classified by the tactic its best move
  would have executed (`gm_analysis::insights::tactic_theme`, mate length from the solution line), else
  `hangingPiece` when the played move left a piece en prise; each one adds 0.35 (capped at 1.4).
Ties and "no data" fall back to a beginner order (hangingPiece, fork, mateIn1, pin, ...). The set focuses on the
3 weakest themes (2 when exactly two of the top three have evidence).

**Building.** Up to 4 own-game positions (focus-theme cards first, then still-learning, recent and costly ones)
sit at slots 3, 6, 9, 12; the rest are pack puzzles shared round-robin over the focus themes, rating from
`puzzle_rating − 100` to `+ 150` (sampled among the 6 nearest), avoiding puzzles attempted in the last 30 days,
topped up from any theme when a focus theme runs dry, ordered by rating. Deterministic for a seed
(FNV of the week key + generation). Items are snapshotted (FEN + solution), so a set never changes; "New set" adds
a newer set for the same week (older results stay in the history). The newest 120 sets are kept.

```
FocusTheme { theme, reason: "games"|"puzzles"|"starter", game_misses /* last 30 days */,
             puzzle_attempts, puzzle_fails /* last 30 days */, score }
WeeklyItem { index, kind: "puzzle"|"mistake", theme /* focus theme, "" if none */,
             puzzle: { id /* pack id or "mistake-<card id>" */, fen, moves: [uci], rating, themes, userFirst? },
             game?: { card_id, game_id?, opponent, move_number, played_san, best_san, classification } /* mistake items */,
             result: null|"solved"|"failed", time_ms? }
WeeklySet { week /* "2026-W41" */, week_start, week_end /* YYYY-MM-DD */, set_id, generation, created_at, rating,
            focus: [FocusTheme], items: [WeeklyItem], progress: { total, done, solved }, finished }
```

| Method & path | Body / query | Response |
|---|---|---|
| `GET /api/weekly` | | `WeeklySet` — this week's set; built and stored on the first request of the week |
| `POST /api/weekly/regenerate` | | `WeeklySet` — "New set": a fresh set for this week (`generation + 1`) |
| `POST /api/weekly/attempt` | `{ set_id, index, solved, time_ms? }` | `{ item, counted, progress, finished, rating? }` — only the first attempt of an item counts (`counted: false` afterwards). A counted pack puzzle is also a rated attempt (`rating` = `PuzzleResult`, activity `puzzle`); a counted own-game item also answers its mistake card (activity `mistake_review`). Unknown set/item → 404, `index ≥ 30` or a malformed body → 400 |
| `GET /api/weekly/history` | `?weeks=1..12` (6) | `{ weeks: [{ week, week_start, progress: {total, done, solved} \| null, themes: [{ theme, attempted, solved }] }] }` — oldest first, the last entry is this week; theme counts cover all sets of the week. Never builds a set |

**Frontend.** `#/puzzles/weekly` (`params.mode === 'weekly'`; `pages/puzzles-weekly.js`, loaded on demand by
`pages/puzzles.js`, which exports `solverKit` for it) shows the overview (progress, item dots, focus themes with a
plain-words reason), the solver (shared `PuzzleRunner`; hints / solution / wrong move = failed; retries never count)
and a finish summary per theme. `components/weekly-card.js` provides `weeklyHubEntry()` (Puzzles hub banner) and
`weeklyInsightsCard()` (Insights card: this week's progress and solve rate per theme over the last 4 weeks), both
`{ el, destroy }`. Styles: `web/css/weekly.css` (`wk-*`, injected on demand). Strings: `weekly` locale namespace.

## Phone & home use

Use GrandMentor from phones and tablets on the same Wi-Fi: HTTPS with a local CA, an access PIN for
every non-loopback client, and the "Use on your phone" card in Settings. Code:
`crates/gm-server/src/phone/` (`tls.rs` certificates, `access.rs` PIN/sessions/limiter, `gate.rs`
middleware, `routes.rs` endpoints, `login.rs` sign-in page, `net.rs` LAN addresses); UI:
`web/js/components/phone.js` (`createPhoneSection() -> { el, destroy }`, styles `web/css/phone.css`,
strings in the `phone` locale namespace). User guide: [`docs/PHONE.md`](PHONE.md).

### Environment

| Variable | Default | Meaning |
|---|---|---|
| `GM_LAN` | unset | `1/true/on/yes` forces phone mode on, `0/false/off/no` off; unset → the saved switch (`phone.json`). |
| `GM_LAN_PORT` | `8443` | HTTPS listener (`0.0.0.0`) in phone mode. |
| `GM_PHONE_DIR` | `<dir of GM_DB>/grandmentor-phone` | `phone.json`, `access.json` (0600), `ca.key.pem` (0600), `ca.crt.pem`, `ca.json`, `server.key.pem` (0600), `server.crt.pem`, `server.json`. |
| `GM_ACCESS_PIN` | on | `off` disables the PIN gate for non-loopback clients. |

In phone mode the plain HTTP listener binds `0.0.0.0:GM_PORT` unless `GM_HOST` is set explicitly.
Nothing is written to `GM_PHONE_DIR` while phone mode is off and the server is loopback-only (except
`phone.json` when the user flips the switch).

### Listeners and the access gate (`phone::gate::gate`, wraps the whole router)

`gm_server::app_with_phone(state, web_dir, Arc<PhoneState>, Transport::Http | Transport::Https)`
builds the router for one listener (`gm_server::app` = in-memory phone state, HTTP). Per request:

1. Loopback peer (`127.0.0.0/8`, `::1`, `::ffff:127.*`) → always allowed.
2. `/phone/ca.crt`, `/favicon.ico` → public.
3. Plain HTTP from another device while phone mode runs → `/api/sync/snapshot` and
   `/api/sync/merge` pass; `GET`/`HEAD` get `307` to `https://<host>:GM_LAN_PORT<path>`; other
   methods `403 {error}`.
4. PIN off, the two sync endpoints, `/login`, `POST /api/access/login`, `/manifest.webmanifest` and
   `/img/icons/*` → allowed (browsers fetch the manifest without cookies).
5. Valid `gm_access` cookie → allowed.
6. Otherwise: `/api/*` (including the websocket) → `401 {"error": "<localized>", "login": "/login"}`;
   page loads (`/`, `*.html`, `Accept: text/html`) → `303` to `/login[?next=<path>]`; anything else
   `401`. The frontend (`api.js`) sends the browser to `/login` on a `401` whose body has
   `"login": "/login"`.

Cookie: `gm_access=<64 hex>; Path=/; Max-Age=31536000; HttpOnly; SameSite=Strict[; Secure on HTTPS]`.
At most 32 signed-in devices are kept (oldest dropped). Wrong PINs: 5 per IP per 15 min, 30 in total
per hour → `429` + `Retry-After`. PIN = 6 digits (spaces/dashes ignored when typed).

### Endpoints

| Method & path | Who | Body | Response |
|---|---|---|---|
| `GET /api/phone/status` | this computer | — | `PhoneStatus` (below) |
| `PUT /api/phone/lan` | this computer | `{ "enabled": bool }` | `PhoneStatus`; saves `phone.json`, takes effect after a restart. `409` when `GM_LAN` is set. |
| `POST /api/phone/pin` | this computer | — | `{ "pin": "123456" }` (new PIN; signed-in devices stay signed in) |
| `DELETE /api/phone/devices` | this computer | — | `{ "signed_out": n, "devices": 0 }` |
| `GET /api/phone/qr?url=<url>` | this computer | — | `image/svg+xml` QR code; `url` must be one of `app_urls`, `ca_urls` or `local_name_url`, else `400` |
| `GET /api/access/status` | any (behind the gate) | — | `{ "local": bool, "signed_in": bool, "pin_required": bool }` |
| `POST /api/access/login` | any | form `pin=…&next=/…` or JSON `{ "pin", "next"? }` (≤ 4 KB) | form: `303` to `next` (same-site path only) + cookie, or the login page with an error (`401` wrong, `429` locked); JSON: `{ "ok": true, "next" }` + cookie, or `401/429 {error}`. A cross-origin `Origin` → `403`. |
| `POST /api/access/logout` | any | — | `{ "ok": true }`, forgets this device's session, clears the cookie |
| `GET /login[?next=/…]` | any | — | self-contained localized HTML sign-in page (no scripts); redirects to `next` when the caller is local or already signed in |
| `GET /phone/ca.crt` | any | — | the local CA (DER, `application/x-x509-ca-cert`, `attachment; filename="grandmentor-ca.crt"`); `404` when phone mode is off |

"This computer" = loopback peer and, when an `Origin` header is sent, a localhost origin; otherwise
`403 {error}` (localized).

```json
PhoneStatus = {
  "lan": { "enabled": true, "running": true, "env": null, "restart_required": false, "can_save": true },
  "http_port": 8080, "https_port": 8443, "network_visible": true,
  "hostname": "chessbox", "addresses": ["192.168.1.20"],
  "app_urls": ["https://192.168.1.20:8443/"],
  "ca_urls": ["http://192.168.1.20:8080/phone/ca.crt"],
  "local_name_url": "https://chessbox.local:8443/",
  "ca": { "fingerprint": "D9:DB:…", "download": "/phone/ca.crt", "names": ["127.0.0.1", "192.168.1.20", "chessbox", "chessbox.local", "localhost"] },
  "pin_required": true, "pin": "123456", "devices": 1
}
```

`app_urls` are `https://` in phone mode, `http://` when only `GM_HOST` opens the network, empty when
the server is loopback-only. `ca` is `null` unless phone mode runs. `addresses`: non-loopback,
non-link-local IPv4, `192.168/16` first, then `10/8`, `172.16/12`, `100.64/10`, others (max 8).

### Certificates (`phone::tls::ensure(dir, &ServerNames) -> CertBundle`)

* CA: ECDSA P-256, 10 years, `CA:TRUE, pathlen:0`, key usage certSign/cRLSign, **name constraints**
  permitting only DNS `local`, `localhost`, the host name at creation, and IPs `10/8`, `172.16/12`,
  `192.168/16`, `100.64/10`, `169.254/16`, `127/8`, `::1`.
* Server: ECDSA P-256, 397 days, `serverAuth`, SANs `localhost`, `127.0.0.1`, `<host>.local`,
  `<host>` (when it matches the CA's) and every permitted LAN IPv4 (max 16). Re-issued when a name
  is missing, the CA changed, or < 30 days remain; checked at startup and every 2 minutes, hot-swapped
  into the TLS listener (rustls, ring provider, ALPN `http/1.1`).

