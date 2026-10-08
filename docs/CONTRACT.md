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
