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
  pub const ALL: [Lang; 2];
  pub fn code(self) -> &'static str;                            // "en" | "es"
  pub fn parse(tag: &str) -> Option<Lang>;                      // "es", "es-MX", "ES_es" -> Es; unsupported -> None
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
  pub opening_name: Option<String> }
pub struct SideStats { pub accuracy: f32, pub estimated_elo: u16,
  pub counts: std::collections::BTreeMap<String, u32> /* classification -> count */ }
pub struct GameReview { pub start_fen: String, pub moves: Vec<MoveReview>,
  pub evals: Vec<Score> /* len = moves+1, evals[0] = start position */,
  pub white: SideStats, pub black: SideStats, pub opening: Option<gm_content::OpeningRef>,
  pub key_moments: Vec<usize> /* plies */, pub summary: String }
pub async fn review_game(pool: &gm_engine::EnginePool, content: std::sync::Arc<gm_content::Content>,
   start_fen: &str, moves: &[String], depth: u8,
   progress: Option<tokio::sync::mpsc::UnboundedSender<f32> /* 0..1 */>, lang: Lang) -> Result<GameReview, String>;
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
pub fn coach_answer(req: &ChatRequest, lang: Lang) -> ChatResponse; // rule-based; routes English and Spanish questions
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
pub struct GameSummary { /* all GameRecord fields except pgn, moves, review_json; plus move_count */ }
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
  pub fn set_lesson_progress(&self, course_id: &str, lesson_id: &str, completed: bool) -> anyhow::Result<()>;
  pub fn get_progress(&self) -> anyhow::Result<Vec<LessonProgress>>; // {course_id, lesson_id, completed, updated_at}
  pub fn stats(&self) -> anyhow::Result<Stats>; // games played/won/lost/drawn, per-bot record, avg accuracy, rating history
}
pub mod pgn { pub fn to_pgn(...) -> String; pub fn parse_pgn(text: &str) -> Result<Vec<ParsedGame>, String>; }
```

---

## 4. HTTP API (gm-server, all JSON, prefix `/api`)

| Method & path | Request | Response |
|---|---|---|
| GET `/api/health` | – | `{ok, version, llm_enabled, engines}` |
| GET `/api/bots` | – | `[BotProfile]` |
| POST `/api/bot/move` | `{bot_id, start_fen, moves:[uci]}` | `BotMove` |
| POST `/api/engine/analyze` | `{fen, depth?, movetime_ms?, multipv?}` (movetime capped 10s) | `SearchInfo` |
| GET `/api/engine/ws` | WebSocket, see below | |
| POST `/api/review` | `{start_fen?, moves:[uci]}` or `{pgn}` or `{game_id}` (+`depth?` default 14) | `GameReview` (if game_id, cached into the game) |
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
| POST `/api/puzzles/rush` | `{score}` | `{best}` |
| GET `/api/courses` | – | `[{id,title,category,level,description,icon,lesson_count, lessons:[{id,title,summary}]}]` |
| GET `/api/courses/:id` | – | `Course` |
| GET `/api/progress` / POST `/api/progress` | POST `{course_id, lesson_id, completed}` | `[LessonProgress]` |
| GET `/api/openings?q=&side=&level=` | – | `[Opening]` (without long fields is ok) |
| GET `/api/openings/:id` | – | `Opening` |
| GET `/api/openings/lookup?fen=` | – | `OpeningMatch` or `null` |
| GET `/api/endgames` | – | `[EndgameDrill]` |
| GET/PUT `/api/profile` | PUT `ProfilePatch` | `Profile` |
| GET `/api/stats` | – | `Stats` |

Errors: HTTP 4xx/5xx with `{error: "message"}` (message in the request language, see below).

**Language.** Every endpoint accepts `?lang=en|es`; otherwise the `Accept-Language` header picks the
first supported language by q-weight (`es-MX,es;q=0.9,en;q=0.8` → `es`); otherwise English. Only
human text changes; JSON shapes, ids, enum values (`classification`, bot `style`/`category`,
puzzle themes), SAN/UCI/FEN and numbers never do. Localized: bot `description`/`greeting` and
move `chat`; mentor `explanation`, position `ideas` and rule-based chat (the LLM is told to answer in
the language; questions are understood in English or Spanish); review `summary`, per-move
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
Hash routing: `#/`, `#/play`, `#/play/:botId`, `#/analysis?fen=..|?game=:id|?pgn=..`,
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
Keys: `boardTheme` (green|brown|blue|purple|gray), `pieceSet` (cburnett|merida|alpha), `sounds` (bool),
`showCoords`, `showLegal`, `animationMs` (number), `showEvalBar`, `autoQueen`, `theme` (dark|light), `moveNotation` (san|figurine).
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
Board validates legality with vendored chess.js (`/vendor/chess.js`), supports drag & click moves,
legal-move dots, promotion picker (or autoQueen), check highlight, last-move highlight, right-click
arrows/circles, smooth animations, sounds via `components/sound.js` (`playSound('move'|'capture'|'check'|'castle'|'promote'|'gameEnd'|'illegal'|'correct'|'wrong'|'notify')`, WebAudio synthesized and pre-rendered once into buffers, no files; `renderSound(name)` returns the rendered `AudioBuffer` for previews/tests). Move sounds play when the piece lands; a dragged piece lifts under the pointer on press, and captured pieces vanish when the attacker lands.
If `onMove` returns `false` the board reverts the move.

**`components/evalbar.js`**: `new EvalBar(el, {orientation})`, `.set(score, opts?)`, `.setOrientation(c)`, `.destroy()`.
`opts` resolves a finished mate (`{mate:0}`): `{mated:'white'|'black'}` or `{fen}` (side to move is mated); without it the bar keeps the side that was already ahead.
**`components/movelist.js`**: `new MoveList(el, {onSelect(ply)})`, `.setMoves([{san, classification?}])`,
`.setCurrent(ply /* 0 = start */)`, `.destroy()`; keyboard ←/→ handled by the page, not the list.
**`components/evalgraph.js`**: `new EvalGraph(el, {onSelect(ply)})`, `.setData(evals:[Score], classifications)`, `.setCurrent(ply)`, `.destroy()`.
**`components/mentor.js`**: `new MentorPanel(el, { getContext: () => ({fen, moves_san, engine_lines}) })`, `.say(text, {kind})`, `.destroy()` — a chat panel that calls `/api/mentor/chat`.
**`components/clock.js`**: `new ChessClock(el, {initialMs, incrementMs, onFlag(color)})`, `.start(color)`, `.press()`, `.pause()`, `.destroy()`.

CSS class vocabulary and tokens are defined in `docs/STYLEGUIDE.md` (written by the design-system owner).

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
