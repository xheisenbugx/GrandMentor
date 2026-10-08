<div align="center">

<img src="docs/media/app-icon.png" width="128" alt="GrandMentor app icon: a crowned knight" />

# GrandMentor

**Your friendly, lightning-fast chess coach — play, review, learn and improve.**

Play charming bots, see who's winning at a glance, get every game reviewed by a mentor,
and learn openings, tactics and endgames — all in one beautiful app that runs on your own computer.

[![Rust](https://img.shields.io/badge/Rust-axum%20%2B%20tokio-b7410e?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Frontend](https://img.shields.io/badge/Frontend-vanilla%20JS%2C%20no%20build-f7df1e?logo=javascript&logoColor=black)](web/)
[![SQLite](https://img.shields.io/badge/Storage-SQLite-003b57?logo=sqlite&logoColor=white)](crates/gm-store)
[![Engine](https://img.shields.io/badge/Engine-~2M%20nodes%2Fs-81b64c)](crates/gm-engine)
[![Puzzles](https://img.shields.io/badge/Puzzles-4%2C200-26c2a3)](data/puzzles.json)
[![Mentor](https://img.shields.io/badge/Mentor-Claude%20optional-d97757?logo=anthropic&logoColor=white)](#-the-mentor)

<br />

<img src="docs/media/play.gif" width="860" alt="Playing a game against Pawnny with live coach feedback, an eval bar and a two-step hint" />

<sub>Playing Pawnny 🐣 in Friendly mode: live coach feedback on every move, the eval bar, and a two-step hint.</sub>

</div>

---

## ✨ Why GrandMentor?

Most chess sites are built for people who already play well. GrandMentor is built for **everyone else**.

- 🧸 **Gentle by default.** Bots greet you, chat with you and blunder like real humans at your level — not like a calculator that suddenly forgets how to play.
- 🎓 **A mentor, not just an engine.** Every move gets a plain-language explanation: *"This hangs your bishop — Qxc4 wins it. Better was d4."*
- ⚡ **Ridiculously fast.** A Rust engine searches ~2 million positions per second; a full game review takes about half a second.
- 🌍 **Speaks your language.** English, Español, Português, Français and Deutsch: every screen, lesson, opening and coach comment.
- 🔒 **Yours.** Runs locally, stores your games in a single SQLite file, no account, no ads, no tracking.

---

## 🎬 Tour

### ♟️ Play against bots with personality

Sixteen opponents from **Pawnny 🐣 (250)** to **Titan 🤖 (3000)**, plus two coach bots who explain their ideas.
Pick your colour, a time control, and how much help you want — *Friendly* (hints, takebacks, eval bar, coach) or *Challenge* (just you and the board).

<p align="center"><img src="docs/media/play-bots.png" width="860" alt="Bot selection screen" /></p>

<table>
<tr>
<td width="50%"><img src="docs/media/play.png" alt="A game in progress with coach feedback and a hint arrow" /></td>
<td width="50%">

**While you play**
- 📊 **Eval bar** shows who's winning, live
- 💡 **Two-step hints** — first the piece, then the move
- 🗣️ **Coach mode** rates each move (*best*, *inaccuracy*, *blunder*…) and says why
- ↩️ Takebacks, draw offers, flip board, clocks from bullet to classical
- 💾 Every game is **saved automatically** — resume unfinished ones anytime

</td>
</tr>
</table>

### 🔍 Game Review — like having a coach watch your game

One click after the game ends. You get **accuracy** for both players, an **estimated rating**, an **eval graph**, **key moments**, and a move-by-move walkthrough with chess.com-style badges:
**brilliant `!!`**, **great `!`**, **best ★**, excellent, good, book 📖, inaccuracy `?!`, mistake `?`, miss, **blunder `??`**.

<p align="center"><img src="docs/media/review.gif" width="860" alt="Stepping through Morphy's Opera Game in Game Review" /></p>
<p align="center"><sub>Walking through the finale of Morphy's famous <i>Opera Game</i> (1858) — badges on the board, coach bubble, synced eval bar.</sub></p>

<table>
<tr>
<td width="50%"><img src="docs/media/review.png" alt="Game review summary with accuracy rings" /></td>
<td width="50%"><img src="docs/media/review-walk.png" alt="Review walkthrough showing the final checkmate" /></td>
</tr>
</table>

### 🧠 Analysis board

A full analysis board with the engine's **top three lines**, best-move arrows, a variation tree, opening names, a position editor, and FEN/PGN import & export.
Stuck? Switch to the **Mentor** tab and ask *"what's the plan here?"*.

<p align="center"><img src="docs/media/analysis.png" width="860" alt="Analysis board with three engine lines at depth 27" /></p>

### 🧩 Puzzles, Puzzle Rush and the Daily Puzzle

**4,200 hand-picked tactics** from the Lichess puzzle database, rated from 400 to 2800 across 70+ themes (forks, pins, mates in 2, back-rank, sacrifices…).
Your **puzzle rating** adapts as you solve. Race the clock in **Puzzle Rush** (3 min, 5 min or Survival — three strikes and you're out), or keep a streak with the **Daily Puzzle**.

<table>
<tr>
<td width="50%"><img src="docs/media/puzzle.gif" alt="Solving the daily puzzle: mate in two" /></td>
<td width="50%"><img src="docs/media/puzzles.png" alt="Puzzle hub with modes and themes" /></td>
</tr>
</table>

### 📚 Learn — from "how does the knight move?" to the Lucena position

**10 courses · 58 interactive lessons.** Each lesson is a short story with arrows and highlights, then *your turn*: find the move on the board. Wrong? Get a gentle nudge and a hint. Right? A little celebration ✨.

<table>
<tr>
<td width="50%"><img src="docs/media/learn.png" alt="Learn hub with courses and progress" /></td>
<td width="50%"><img src="docs/media/lesson.png" alt="A lesson on knight forks" /></td>
</tr>
</table>

### 📖 Openings & ♚ Endgames

- **~280 named openings** — ideas, traps, popularity, an **explorer** of book moves, and a **trainer** that quizzes you on your repertoire from memory.
- **41 endgame drills** — basic mates, opposition and key squares, the rule of the square, Lucena, Philidor, Vancura, queen vs pawn — each with a short lesson, then practised against the full-strength engine from several starting positions until you've **mastered** it (3 successes in a row). Every position is checked against tablebases.

<table>
<tr>
<td width="50%"><img src="docs/media/openings.png" alt="Italian Game opening page" /></td>
<td width="50%"><img src="docs/media/endgames.png" alt="Endgame drills grid" /></td>
</tr>
</table>

### 🏠 Home, Library & Profile

Your dashboard, every game you've ever played (search, favourites, notes, PGN import/export), and your progress: win rate, accuracy trend, puzzle-rating history, record vs each bot, streaks and achievements.

<table>
<tr>
<td width="33%"><img src="docs/media/home.png" alt="Home dashboard" /></td>
<td width="33%"><img src="docs/media/library.png" alt="Game library" /></td>
<td width="33%"><img src="docs/media/profile.png" alt="Profile and stats" /></td>
</tr>
</table>

### 📅 Today's plan, streaks & goals

Home suggests a 10–15 minute session built from what you need today: the daily puzzle, mistakes that are due, a repertoire review, the next lesson, a quick drill or a game against a bot near your level. Pick a daily goal, keep your streak going, and watch your 12-week activity calendar fill up.

<table>
<tr>
<td width="55%"><img src="docs/media/prs/feat-learning-platform/daily-plan.png" alt="Today's plan on the Home page with tasks, goal ring and streak" /></td>
<td width="45%"><img src="docs/media/prs/feat-learning-platform/daily-goal-met.gif" alt="Completing the daily goal" /></td>
</tr>
</table>

### 📈 Insights — your top 3 weaknesses

GrandMentor reads all your reviewed games and tells you, in plain words, what costs you the most points: pieces you leave hanging, tactics you miss, endgames you let slip, winning positions you don't convert. Every weakness has a **one-click drill** to fix it.

<img src="docs/media/prs/feat-learning-platform/insights-page.png" width="860" alt="Insights page with top weaknesses and charts" />

### 🔁 Learn from your mistakes

Every blunder and missed win from your own games becomes a "find the better move" puzzle. They come back on a **spaced-repetition** schedule until you get them right three times in a row.

<table>
<tr>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/mistakes-solve.gif" alt="Solving a puzzle taken from your own game" /></td>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/mistakes-hub.png" alt="The mistakes deck on the Puzzles page" /></td>
</tr>
</table>

### 📘 Your opening repertoire

Build your own lines for White and Black (or start from a beginner-friendly starter set), then drill them from memory: the bot plays the sidelines, you answer. After each game GrandMentor tells you exactly where you left your preparation.

<table>
<tr>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/repertoire-tree.png" alt="Repertoire tree for White" /></td>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/repertoire-drill.gif" alt="Repertoire drill with spaced repetition" /></td>
</tr>
</table>

### ⚡ Quick drills & interactive lessons

Thirty-second board-vision games with personal bests (find the square, spot the hanging piece, count the material, find all checks, knight routes), plus new lesson types: **guess the master's move**, count the material, spot the hanging piece and pick the best plan.

<table>
<tr>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/drills-coordinates.gif" alt="Coordinates drill" /></td>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/lessons-tasks.gif" alt="Interactive lesson tasks" /></td>
</tr>
</table>

### 🏛️ Classic games, narrated

27 of the most famous games ever played, from Morphy's Opera Game to Kasparov's Immortal and Deep Blue, narrated move by move by the mentor, with "pause and think" moments where you guess the key move.

<table>
<tr>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/classics-library.png" alt="Classic games library" /></td>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/classics-player.gif" alt="Narrated classic game" /></td>
</tr>
</table>

### 🎮 More ways to play

- **Sparky ⚡, the adaptive bot**, gets stronger when you win and gentler when you lose, and your **estimated rating** updates after every game.
- **Premoves**, an optional **"confirm move"** step for beginners, **typed moves** (`Nf3`, `O-O`) and a **blindfold** mode.
- **Play from any position**: take over a game from Game Review, the analysis board, an endgame or a classic game.
- **Play a friend** on the same device, with clocks, takebacks, draw offers and an automatic Game Review afterwards.

<table>
<tr>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/play-premove.gif" alt="Premoves against a bot" /></td>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/local-pass-and-play.gif" alt="Two players on one device" /></td>
</tr>
</table>

### 📲 Install it, use it offline, back it up

Install GrandMentor like a native app on your phone or computer. Puzzles and lessons keep working **offline**, and your results sync when you're back. One click downloads a backup of everything; restore it on another machine, or sync two devices on your network with a pairing code.

<table>
<tr>
<td width="25%"><img src="docs/media/prs/feat-learning-platform/pwa-offline-mobile.png" alt="A lesson working offline on a phone" /></td>
<td width="75%"><img src="docs/media/prs/feat-learning-platform/backup-section.png" alt="Backup and sync settings" /></td>
</tr>
</table>

### ♿ Accessible to everyone

Play entirely with the keyboard (arrow keys + Enter, or type `Nf3`), hear every move with a screen reader, and switch on high contrast, colour-blind-friendly move colours, reduced motion or larger text in Settings.

<table>
<tr>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/a11y-keyboard.gif" alt="Playing with the keyboard, with square names and screen-reader announcements" /></td>
<td width="50%"><img src="docs/media/prs/feat-learning-platform/a11y-high-contrast.png" alt="High-contrast mode with colour-blind-friendly move colours" /></td>
</tr>
</table>

### 🌍 Five languages

<table>
<tr>
<td width="33%"><img src="docs/media/prs/feat-learning-platform/i18n-pt-home.png" alt="Home in Portuguese" /></td>
<td width="33%"><img src="docs/media/prs/feat-learning-platform/i18n-fr-play.png" alt="Play setup in French" /></td>
<td width="33%"><img src="docs/media/prs/feat-learning-platform/i18n-de-review.png" alt="Game Review in German" /></td>
</tr>
</table>

### 🎨 Light mode, board themes & mobile

Dark or light, five board colours, three piece sets, and a layout that works just as well on your phone.

<table>
<tr>
<td width="56%"><img src="docs/media/settings-light.png" alt="Settings in light mode" /></td>
<td width="22%"><img src="docs/media/mobile-home.png" alt="Home on mobile" /></td>
<td width="22%"><img src="docs/media/mobile-review.png" alt="Game review on mobile" /></td>
</tr>
</table>

---

## 🎓 The Mentor

GrandMentor's coach works **out of the box** with an instant, rule-based explainer that spots hanging pieces, forks, pins, missed mates, development and king safety.

Want a deeper conversation? Set an Anthropic API key and the mentor chat is powered by **Claude**, grounded in the engine's analysis of your exact position (it never invents moves):

```bash
export ANTHROPIC_API_KEY=sk-ant-...
cargo run --release -p gm-server
```

---

## 🚀 Quick start

You only need a recent stable **Rust** toolchain (tested with 1.98). No Node, no database server.

```bash
git clone https://github.com/xheisenbugx/GrandMentor.git
cd GrandMentor
cargo run --release -p gm-server
# → open http://localhost:8080
```

Run from the repository root so `./data` and `./web` are found. Your games live in `./grandmentor.db`, created on first run.

<details>
<summary><b>⚙️ Configuration (environment variables)</b></summary>

| Variable | Default | Meaning |
|---|---|---|
| `GM_PORT` | `8080` | HTTP port |
| `GM_HOST` | `127.0.0.1` | Bind address. `0.0.0.0` exposes it on your LAN — there is no authentication. |
| `GM_DATA_DIR` | `./data` | Content JSON: openings, puzzles, courses, endgames |
| `GM_DB` | `./grandmentor.db` | SQLite database file (WAL mode) |
| `GM_WEB_DIR` | `./web` | Static frontend directory |
| `GM_ENGINES` | cores − 1, clamped to 2–8 | Engines in the search pool, one thread each |
| `GM_TT_MB` | `32` | Hash table per engine (MB). Memory ≈ `GM_ENGINES × GM_TT_MB`. |
| `GM_SYNC_ALLOW_ORIGINS` | – | Extra origins allowed to sync with this device (pairing code still required) |
| `ANTHROPIC_API_KEY` | – | Enables the Claude-powered mentor chat |
| `GM_MENTOR_MODEL` | `claude-opus-5-5` | Claude model used by the mentor |
| `RUST_LOG` | `info` | Log filter |

</details>

<details>
<summary><b>🧪 Tests & lints</b></summary>

```bash
cargo test --workspace --release
cargo clippy --workspace --all-targets
cargo test -p gm-bots --release -- --ignored   # slow bot-vs-bot matches
for f in $(find web/js -name '*.js'); do node --input-type=module --check < "$f"; done
```

</details>

---

## 🏗️ How it's built

```mermaid
flowchart LR
  subgraph Browser["🌐 Browser — vanilla ES modules, no build step"]
    UI["Pages<br/>play · review · analysis · puzzles · learn"]
    Board["Board · EvalBar · MoveList<br/>EvalGraph · Mentor chat"]
  end
  subgraph Server["🦀 grandmentor (Rust · axum)"]
    API["REST /api/*"]
    WS["WebSocket<br/>/api/engine/ws"]
    Engine["gm-engine<br/>PVS · TT · LMR · PeSTO"]
    Bots["gm-bots"]
    Review["gm-analysis"]
    Mentor["gm-mentor"]
    Store["gm-store<br/>SQLite"]
    Content["gm-content<br/>openings · puzzles · lessons"]
  end
  Claude(("Claude API<br/>optional"))
  UI --> API
  Board <--> WS
  API --> Bots & Review & Mentor & Store & Content
  WS --> Engine
  Bots & Review --> Engine
  Mentor -.-> Claude
```

| Crate | What it does |
|---|---|
| [`gm-engine`](crates/gm-engine) | Alpha-beta (PVS) engine on `shakmaty`: iterative deepening, aspiration windows, transposition table, null move, LMR, killers/history, quiescence with SEE, tapered PeSTO-style eval. **~2M nodes/s, depth 18–20 in 1 s.** |
| [`gm-bots`](crates/gm-bots) | Bot personas and human-like move choice (depth/node limits, softmax over MultiPV, style bias, opening book) |
| [`gm-analysis`](crates/gm-analysis) | Game review: parallel per-ply analysis, lichess-style win%/accuracy, chess.com-style classifications, key moments |
| [`gm-mentor`](crates/gm-mentor) | Rule-based move explanations and position insights, plus optional Claude chat |
| [`gm-store`](crates/gm-store) | SQLite: games, profile, puzzle rating, lesson progress, stats, activity and streaks, spaced-repetition decks (mistakes, repertoire), drills, endgame mastery, adaptive rating, backup/merge; PGN import/export |
| [`gm-content`](crates/gm-content) | Loads and validates every opening, puzzle, lesson and drill (every move is checked for legality) |
| [`gm-server`](crates/gm-server) | The `grandmentor` binary: REST + WebSocket + compressed static files |

The full API and JSON contract lives in [`docs/CONTRACT.md`](docs/CONTRACT.md).

### Fast and leak-free by design

- **Bounded memory** — fixed-size hash tables, capped LRU caches, size-limited requests, capped search times.
- **Never blocks** — searches and SQLite calls run on blocking threads; the async runtime stays responsive.
- **Always stoppable** — every search has a stop flag, raised on new requests, socket close or shutdown; engines always return to the pool.
- **Clean frontend** — every page and component tears down its listeners, timers, observers, fetches and sockets. A QA pass routed through every page 10+ times with flat listener/socket/timer counts.

---

## 🤝 Contributing

Contributions are welcome! Please read [`AGENTS.md`](AGENTS.md) — it describes the branch flow (PRs go into **`dev`**), the PR template (including the **"In plain words"** section and visual evidence), and the project conventions. It applies to humans and AI agents alike.

Before opening a PR, run the quality gate (it also runs in CI on every PR):

```bash
node tools/qa/check-i18n.mjs     # every language has every key
node tools/qa/sweep.mjs          # every page × language × desktop/mobile in headless Chrome
node tools/qa/bench-gate.mjs     # engine speed and tactics (for engine changes)
```

---

## 🙏 Credits

- **Puzzles** — [Lichess puzzle database](https://database.lichess.org/#puzzles) (CC0). See [`data/ATTRIBUTION.md`](data/ATTRIBUTION.md).
- **Pieces** — from [lichess-org/lila](https://github.com/lichess-org/lila): *cburnett* (Colin M.L. Burnett, GPLv2+), *merida* (Armando Hernandez Marroquin, GPLv2+), *alpha* (Eric Bentzen — personal, non-commercial use only). See [`web/img/pieces/LICENSE.md`](web/img/pieces/LICENSE.md).
- **[chess.js](https://github.com/jhlywa/chess.js)** v1.4.0 (BSD-2-Clause) and **[shakmaty](https://github.com/niklasf/shakmaty)** (GPL-3.0+).
- Opening names follow the standard ECO classification. The *Opera Game* in the screenshots is Morphy vs. Duke Karl / Count Isouard, Paris 1858.

<div align="center">
<br />
<sub>GrandMentor is an independent project and is not affiliated with chess.com or lichess.org.</sub>
<br /><br />
<b>♞ Made with love for every player who has ever hung their queen. ♞</b>
</div>
