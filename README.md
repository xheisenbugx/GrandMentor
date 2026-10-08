# GrandMentor

A fast, local-first chess learning app modeled on chess.com. You can play friendly bots, review your games with a coach, solve tactics puzzles, take lessons, drill openings and endgames, and keep every game in a personal library.

The backend is a single Rust binary: an axum web server with its own alpha-beta engine, SQLite storage and a rule-based coach, plus an optional Claude-powered mentor. The frontend uses plain ES modules with no build step.

```
cargo run --release -p gm-server      # then open http://localhost:8080
```

---

## Features

### Play
- **16 bots** from Pawnny (250) to Titan (3000), plus two coach bots (Coach Mia 1000, Coach Leo 1700). Each bot has its own personality, avatar, greeting and chat lines. Weak bots blunder plausibly (shallow search, softmax over multi-PV, a style bias) rather than at random, and every bot uses the opening book.
- **Assisted modes:** Friendly (hints, takebacks, eval bar and coach) or Challenge (no help), or pick each option yourself.
- **Two-step hints**, like chess.com: the first press highlights the piece to move and the second shows the move.
- **Live coach feedback** rates each of your moves: brilliant, great, best, excellent, good, book, inaccuracy, mistake, miss or blunder.
- **Time controls** (or no clock), resign, draw offer, flip board, resume an unfinished game.
- **Game-over card** with the result, the opening, and buttons for **Game Review**, Rematch and New bot. **Every game is saved automatically.**

### Analyze
- **Game Review**, as on chess.com:
  - per-side accuracy and estimated game rating
  - classification counts and an eval graph with coloured dots
  - key moments, then a move-by-move walkthrough with a coach bubble, square badges and the best line
- **Analysis board:**
  - live engine with 3 lines over a WebSocket, depth and speed display, best-move arrow
  - **eval bar** showing who is winning
  - move tree with variations, opening name and book moves
  - FEN/PGN load, position setup, copy, save
- **Mentor chat** on the review and analysis pages. Ask "what's the plan?" or "why was this a mistake?". It uses Claude when `ANTHROPIC_API_KEY` is set and falls back to an instant rule-based coach otherwise.

### Learn
- **Puzzles:** about 4,200 rated tactics from the Lichess database.
  - Glicko-style puzzle rating, with themes and difficulty filters
  - hint, solution and retry
  - **Daily Puzzle** with a streak
  - **Puzzle Rush** in 3 min, 5 min or Survival mode, with three strikes
- **Lessons:** 10 courses and 58 lessons (basics, openings, middlegame, tactics, strategy, endgames). Interactive steps use arrows and highlights, and include "your turn" tasks. Progress is saved.
- **Openings:** about 280 named openings with ideas, traps and popularity. Learn, explore and train each one, with spaced-repetition training of your repertoire from memory.
- **Endgame drills:** 36 drills (basic mates, pawn, rook, minor-piece and queen endings). You play the position against the full-strength engine until you win or hold the draw, with a hint and technique notes for each.

### Your stuff
- **Library:** search, filter, sort, favourites, notes and tags. Import multi-game PGN (opening names are detected automatically), download PGN, delete.
- **Profile:** games, win rate, average accuracy, puzzle-rating history, record against each bot, day streak and 27 achievements.
- **Settings:**
  - dark or light theme
  - 5 board colours and 3 piece sets
  - coordinates, legal-move dots, auto-queen, animation speed
  - SAN or figurine notation, eval bar, sounds (synthesized with WebAudio, no audio files)

### What the screens look like
- **Home:** a greeting banner with your streak and puzzle rating, quick-start cards (Play a bot, Puzzle of the day, Continue learning, Analysis board), a row of bot avatars you can scroll sideways, recent games and a mini board with the daily puzzle.
- **Play:** the board fills the left side, with the bot's avatar and speech bubble above it and an eval bar along its edge. The right panel holds the opening name, a coach feedback bubble, the move list with classification icons and big Hint / Takeback / Flip / Draw / Resign buttons.
- **Game Review:** an eval graph at the top of the panel, a coach summary and two accuracy rings. The classification table comes next, then Start Review, which steps through the game move by move with badges drawn on the board.
- **Mobile (390px):** the sidebar becomes a bottom tab bar, the board is full width, and panels stack under it. From 861 to 1100px wide, the sidebar collapses to icons.

---

## Quick start

Requirements: a recent stable Rust toolchain (tested with 1.98). No Node and no database server are needed.

```bash
git clone <this repo> GrandMentor && cd GrandMentor
cargo run --release -p gm-server
# → GrandMentor is running → open http://localhost:8080
```

Run the commands from the repository root so the default `./data` and `./web` paths resolve.
The SQLite database (`./grandmentor.db`) is created on first run.

To enable the LLM mentor (optional):

```bash
export ANTHROPIC_API_KEY=sk-ant-...
cargo run --release -p gm-server
```

Tests, lints and the engine match tests:

```bash
cargo test --workspace --release
cargo clippy --workspace --all-targets
cargo test -p gm-bots --release -- --ignored   # slow bot-vs-bot matches
```

### Environment variables

| Variable | Default | Meaning |
|---|---|---|
| `GM_PORT` | `8080` | HTTP port |
| `GM_HOST` | `127.0.0.1` | Bind address. Use `0.0.0.0` to expose it on your LAN; there is no authentication. |
| `GM_DATA_DIR` | `./data` | Content JSON: openings, puzzles, courses, endgames |
| `GM_DB` | `./grandmentor.db` | SQLite database file (WAL mode) |
| `GM_WEB_DIR` | `./web` | Static frontend directory |
| `GM_ENGINES` | cores − 1, clamped to 2–8 | Number of engines in the search pool, each on its own thread |
| `GM_TT_MB` | `32` | Transposition-table size per engine, in MB. Memory is about `GM_ENGINES × GM_TT_MB`. |
| `ANTHROPIC_API_KEY` | – | Turns on the Claude-powered mentor chat. Without it, the rule-based coach answers. |
| `GM_MENTOR_MODEL` | `claude-opus-5-5` | Claude model used by the mentor |
| `RUST_LOG` | `info` | Log filter (tracing `EnvFilter`) |

---

## Architecture

```
crates/
  gm-engine/    PVS alpha-beta engine on shakmaty move generation.
                Iterative deepening, aspiration windows, TT, null move, LMR, killers/history,
                quiescence search with SEE, check extensions, tapered PeSTO-style evaluation,
                mate and draw detection. EnginePool hands out engines on blocking threads.
                About 2M nodes/s; depth 18–20 in 1s from the start position.
  gm-content/   Loads and validates data/*.json; opening lookup by position and book moves
  gm-bots/      Bot personas and human-like move choice (depth/nodes limits, softmax over MultiPV)
  gm-analysis/  Game review: per-ply parallel analysis, lichess-style win%/accuracy,
                chess.com-style classifications, key moments, summary
  gm-mentor/    Rule-based explanations and position descriptions, plus optional Claude chat
  gm-store/     SQLite (rusqlite, bundled): games, profile, puzzle rating, progress, stats;
                PGN import and export
  gm-server/    axum binary `grandmentor`: REST /api/*, WebSocket /api/engine/ws,
                static files with gzip/br compression and cache headers, SPA fallback
data/           openings.json  puzzles.json  courses.json  endgames.json
web/            index.html, css/, js/app.js (hash router), js/api.js (REST + EngineClient),
                js/components/ (board, evalbar, movelist, evalgraph, mentor, clock, sound),
                js/pages/ (one module per route), vendor/chess.js, img/pieces/
docs/           CONTRACT.md (API and JSON contract), FEATURES.md, STYLEGUIDE.md
```

`docs/CONTRACT.md` is the full integration contract: every endpoint, JSON shape and component API.

**Speed and memory:**
- **Bounded memory:**
  - every engine has a fixed-size transposition table
  - server caches (reviews, position insights, opening lookups) are capped LRUs
  - request bodies are size-limited, and movetime is capped at 10s over REST and 30s over the WebSocket
- **Nothing blocks the async runtime:** searches and SQLite calls run on blocking threads.
- **Searches can always be stopped:** every search takes an `AtomicBool` stop flag. A new `analyze`, a `stop` message, a socket close or server shutdown raises it, and a drop guard returns the engine to the pool.
- **Frontend cleanup:** every page's `mount()` returns a cleanup function. Through a disposables bag it removes global listeners, timers, ResizeObservers, fetches (via AbortController), WebSockets and components, and every component has `destroy()`. The QA pass routed through all pages 10+ times and the counts of global listeners, intervals, sockets and observers stayed flat.

---

## Attribution & licenses

- **Puzzles:** the [Lichess puzzle database](https://database.lichess.org/#puzzles), CC0 1.0. Puzzle ids keep the Lichess id after the `lc_` prefix (`https://lichess.org/training/<id>`). See `data/ATTRIBUTION.md`.
- **Pieces:** copied unmodified from [lichess-org/lila](https://github.com/lichess-org/lila). See `web/img/pieces/LICENSE.md`.
  - **cburnett** by Colin M.L. Burnett, GPLv2+
  - **merida** by Armando Hernandez Marroquin, GPLv2+
  - **alpha** by Eric Bentzen, free for personal, non-commercial use only. Remove it if you ever distribute GrandMentor commercially.
- **[chess.js](https://github.com/jhlywa/chess.js)** v1.4.0 by Jeff Hlywa, BSD-2-Clause, vendored at `web/vendor/chess.js`.
- **[shakmaty](https://github.com/niklasf/shakmaty)** (GPL-3.0+) supplies move generation for the Rust engine.
- The opening names follow the standard ECO classification.

GrandMentor is not affiliated with chess.com or lichess.org.
