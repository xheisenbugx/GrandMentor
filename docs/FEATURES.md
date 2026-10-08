# GrandMentor: Feature Inventory & UX Spec

This document records what chess.com (plus some lichess features) offers and how each feature maps
onto GrandMentor routes and components from `docs/CONTRACT.md`. It also lists the concrete UX details
builders should copy. Where this file and CONTRACT.md disagree on an API or shape, **CONTRACT.md wins**.
This file only describes behaviour and look.

Priorities: **P0** = needed for a credible v1, **P1** = adds a lot of value and should ship if time allows,
**P2** = nice to have.

Sources: chess.com support articles "How does Game Review work" and "How are moves classified",
the chess.com news posts "Game Review v2", "Announcing the updated Puzzle Rush" and "Puzzle Battle",
lichess.org/learn, /practice, /training/coordinate and the lichess opening explorer. I also used direct
knowledge of the current chess.com UI. Exact hex colours come from chess.com's public icon set.

---

## 1. chess.com feature inventory

### 1.1 Play
| Feature | What it is / how it works UX-wise |
|---|---|
| **Play vs Computer (bots)** | A grid of bot avatars grouped by tier (Beginner, Intermediate, Advanced, Master, Athletes/Personalities, Engine). Selecting a bot shows a large portrait, name, rating, flag, a 1-line bio and its speech bubble greeting. A big green **Play** button sits at the bottom. Before starting you pick your colour (white / random / black) and optionally the time control. |
| **Bot personalities & chat** | Bots talk in a speech bubble next to their avatar: a greeting, reactions to captures/blunders ("Ouch!") and a game-end line. Each style (aggressive, defensive, trappy) is reflected in move choice. |
| **Assisted modes** | Toggles on the setup screen: *Challenge* (no help), *Friendly* (hints, takebacks, eval), *Custom* (pick each): **Hints**, **Takebacks**, **Evaluation bar**, **Threats** (red arrows on opponent threats), **Suggestion arrows**, **Show engine lines**. |
| **Hint** | A light-bulb button. 1st press highlights the piece to move (square glow). 2nd press shows the full move as an arrow. Using a hint marks the game "assisted". |
| **Takeback** | Undo button that rewinds your move and the bot's reply. |
| **Clocks / time controls** | Bullet, blitz, rapid, daily, plus no clock vs bots by default. Clock turns red under 10s (20s for longer controls), shows tenths below 10s and plays a low-time tick. |
| **Game over modal** | A big centred card: result headline ("You Won!" / "Martin Won"), how it ended ("by checkmate"), both avatars, a mini accuracy summary, and buttons **Game Review** (primary, green), **New Game**, **Rematch**. Confetti on a win. |
| **Play vs friend / online** | Live and correspondence games (out of scope: GrandMentor is local-first). Local "pass-and-play" two-humans mode is a cheap stand-in. |
| **Premoves, move confirmation, auto-queen** | Settings toggles. |

### 1.2 Analysis & Review
| Feature | What it is / how it works UX-wise |
|---|---|
| **Game Review** | After a game a coach avatar (a "coach" persona) shows a **summary report**: a speech-bubble sentence, an eval graph (white area over black area) with classification dots, both players' **accuracy** (big number, 0–100), **Game Rating** (estimated performance Elo per side), and a **classification count table** (Brilliant … Blunder rows, white count left, black count right, each row with its coloured icon). Then a **Start Review** button walks you move by move. |
| **Move-by-move review** | For each move: the coach bubble says e.g. "**Nf3 is a mistake**. This allows a fork." with the classification icon at the start. The board shows the played move with a classification **badge on the destination square** (a coloured circle with a symbol in the top-right corner of the square). Buttons: **Best** (shows the best move as a green arrow and badge), **Retry** (try to find the better move yourself), **Next** (continue). Eval bar animates with each step. |
| **Key moments** | Review can jump only between important plies (mistakes, blunders, misses, brilliants, greats) using "Next key moment" / "Previous". |
| **Retry mistakes** | Before/after the review the user gets "Retry your mistakes": each mistake/blunder position becomes a mini-puzzle ("Find the better move"). |
| **Classification V2** | Uses an Expected Points model (win probability adjusted for rating). Labels: Brilliant, Great, Best, Excellent, Good, Book, Inaccuracy, Mistake, Miss, Blunder (+ Forced = only legal move). **Miss** = failed to punish the opponent's error (replaced "missed win"). |
| **Accuracy (CAPS)** | 0–100 per side. GrandMentor uses the lichess-style win% formula (see CONTRACT). |
| **Analysis board** | Free board with engine on/off toggle, 1–5 engine lines (score + SAN preview, click to play), depth display, eval bar, move tree with variations, opening name, explorer tab, FEN/PGN load/paste, board flip, share/download PGN. Arrow keys step through moves. |
| **Eval bar** | A vertical bar to the left of the board (see §3.2). |
| **Evaluation graph** | Area chart below/beside the move list, clickable to jump to a ply, with coloured dots for mistakes/blunders/brilliants. |
| **Coach explanations** | Plain-language sentences generated per move ("You left your knight on f3 undefended", "This develops a piece and controls the centre"). Shown in a speech bubble from a coach avatar. |
| **Self-analysis / Insights** | Aggregated stats: accuracy by phase (opening/middlegame/endgame), win rate by colour, best openings, time-usage. |

### 1.3 Puzzles
| Feature | What it is / how it works UX-wise |
|---|---|
| **Puzzles (rated)** | Board loads, opponent's last move is animated after ~0.5s, a banner says "White to Move" / "Black to Move". Correct move → green square highlight + check-mark badge + "correct" sound, the opponent replies automatically. Wrong move → red highlight + X badge + "wrong" sound, move is reverted; the puzzle is counted as failed (rated mode) but you may keep trying. On finish: rating change shown as `+8` (green) or `-12` (red) next to the puzzle rating. Buttons: **Hint**, **Solution**, **Retry**, **Next**, **Analyze**. |
| **Puzzle Rush** | Modes: **3 min**, **5 min**, **Survival** (no time limit). **Three strikes and you're out** in every mode. Puzzles start easy and get harder. Score counter is big; each solved puzzle adds a green tile, each miss a red X tile in a row of results. Personal best is shown, and the end screen lists every puzzle (green/red) and lets you click any one to review it. Rush does not change the puzzle rating. |
| **Puzzle Battle** | Head-to-head 3-minute rush vs another player (P2: could be done vs a simulated bot score). |
| **Daily Puzzle** | One curated puzzle per day, same for all users, with streak tracking. |
| **Custom / Themes** | Choose themes (fork, pin, mate in 2, endgame…) and difficulty. |
| **Learning mode** | Unrated puzzles where you can't lose rating; wrong moves get explanations. |

### 1.4 Learn
| Feature | What it is / how it works UX-wise |
|---|---|
| **Lessons** | Courses grouped by level and category (Beginner/Intermediate/Advanced; Openings, Strategy, Tactics, Endgames). Each lesson = a short video + **interactive challenges**. A step panel shows text, the board shows arrows/highlights, a progress bar along the top, and "Next". Challenges say "Find the best move for White" and use puzzle-style feedback. Completed lessons get a green check; course cards show % complete. |
| **Openings (explorer + courses)** | Opening explorer: for the current position, a table of next moves with game count and white/draw/black percentage bars. Openings library page: list of named openings with board thumbnail, ECO, and "Learn" / "Practice" buttons. Practice = play the line from memory against the book; deviation gets a nudge. |
| **Endgame practice / drills** | Positions categorised (basic mates, pawn, rook, minor piece, queen). Goal shown ("Win" / "Draw"). You play against the engine at full strength; success when goal reached. Stars/checkmarks for completed drills. |
| **Coach (Dr. Wolf / Coach bot)** | Plays games with you and explains each move in real time. |
| **Vision / Coordinates (chess.com "Vision", lichess "Coordinates")** | Square-name trainer: a square name is shown; click it on the board within 30s; score counter. |

### 1.5 Library, profile, settings
| Feature | What it is / how it works UX-wise |
|---|---|
| **Game archive** | Table of past games: result icon (green +, red −, grey =), opponent, accuracy, moves, date, opening. Filters (result, colour, opponent, time control), search, a "Review" link per row. |
| **Library / collections** | Saved games, notes, favourites (star), tags/collections. PGN import/export. |
| **Stats** | Rating graphs per mode, wins/losses/draws donut, puzzle rating history, rush best, lessons completed, streak. |
| **Settings** | Board theme (green default `#eeeed2`/`#769656`, brown, blue, purple, gray…), piece set, sounds, coordinates (inside/outside/off), highlight moves, show legal moves, animation speed (none/slow/medium/fast), auto-queen, notation (SAN/figurine), dark/light theme. Live preview board inside the settings page. |

### 1.6 lichess features worth copying
- **Learn (lichess.org/learn)**: tiny gamified stages for absolute beginners ("Capture all the stars with the rook"), 3 stars per stage depending on move count. This is the best onboarding for people who have never played → P1 as a "Basics" course in `courses.json`.
- **Practice**: themed positions you play out against Stockfish with a goal (e.g. "Checkmate with K+R", "Hold the draw"). This is the same model as our endgame drills.
- **Coordinates trainer**: find the square, 30s timer, white/black orientation → P1.
- **Opening explorer**: move table with W/D/B bars and named openings. We do this from `openings.json` book continuations (no masters DB).
- **Analysis board**: computer arrows, multi-PV, "Learn from your mistakes" (same as chess.com Retry).

---

## 2. Mapping to GrandMentor

| chess.com / lichess feature | GrandMentor route | Components / API | Prio |
|---|---|---|---|
| Home dashboard (Play, Puzzles, Learn tiles, daily puzzle, continue lesson, last game) | `#/` | `home.js`, `/api/puzzles/daily`, `/api/progress`, `/api/games?limit=3`, `/api/stats` | P0 |
| Bot selection screen | `#/play` | `play.js`, `/api/bots` | P0 |
| Play vs bot (board, chat bubble, clocks optional) | `#/play/:botId` | `Board`, `ChessClock`, `EvalBar` (if assisted), `MoveList`, `/api/bot/move` | P0 |
| Hints (2-stage) | `#/play/:botId` | `/api/engine/analyze` (multipv 1), `Board.setHighlights`/`setArrows` | P0 |
| Takebacks | `#/play/:botId` | client-side history | P0 |
| Assisted toggles (eval bar, threats, suggestion arrows) | `#/play/:botId` | `EngineClient`, settings `showEvalBar` | P1 |
| Auto-save finished games | `#/play/:botId` | `POST /api/games` | P0 |
| Game-over modal with "Game Review" CTA | `#/play/:botId` | `ui.modal` | P0 |
| Game Review summary (accuracy, est. rating, counts table, coach summary) | `#/review/:gameId` | `review.js`, `POST /api/review {game_id}`, `EvalGraph`, `MentorPanel` | P0 |
| Move-by-move review with badges, Best / Retry / Next | `#/review/:gameId` | `Board.setBadge`, `setArrows`, `MoveList` (classification), `EvalBar` | P0 |
| Key moments navigation | `#/review/:gameId` | `GameReview.key_moments` | P0 |
| Retry mistakes | `#/review/:gameId` | client logic over `moves[]` where cls ∈ {mistake, blunder, miss} | P1 |
| Review progress bar while analysing | `#/review/:gameId` | (HTTP is one-shot: use an indeterminate progress bar plus friendly "Coach is analysing…" text) | P0 |
| Analysis board (engine lines, eval bar, FEN/PGN load, flip) | `#/analysis` | `analysis.js`, `EngineClient` (multipv 3), `EvalBar`, `MoveList` | P0 |
| Variations / move tree | `#/analysis` | page-local tree; `MoveList` shows the mainline | P2 |
| Mentor chat ("Ask the coach") | `#/analysis`, `#/review/:gameId` | `MentorPanel`, `/api/mentor/chat` (`source` llm or coach) | P0 |
| Position ideas ("What's the plan?") | `#/analysis` | `/api/mentor/position` | P1 |
| Explain this move | `#/analysis`, `#/review` | `/api/mentor/explain` | P1 |
| Opening name display | analysis, play, review | `/api/openings/lookup` | P0 |
| Rated puzzles + rating delta | `#/puzzles` | `puzzles.js`, `/api/puzzles/next`, `/api/puzzles/:id/attempt` | P0 |
| Puzzle themes filter | `#/puzzles` | `/api/puzzles/themes` | P1 |
| Daily puzzle | `#/puzzles/daily` | `/api/puzzles/daily`, profile `streak_days` | P0 |
| Puzzle Rush (3/5 min, Survival, 3 strikes) | `#/puzzles/rush` | `/api/puzzles/rush`, `POST /api/puzzles/rush` | P0 |
| Puzzle Battle vs bot | `#/puzzles/rush` (mode) | simulated opponent score | P2 |
| Courses list (by category: basics/openings/middlegame/tactics/strategy/endgame) | `#/learn` | `learn.js`, `/api/courses`, `/api/progress` | P0 |
| Course detail (lessons + checkmarks) | `#/learn/:courseId` | `/api/courses/:id` | P0 |
| Interactive lesson step player | `#/learn/:courseId/:lessonId` | `lesson.js`, `Step.task`, `POST /api/progress` | P0 |
| Openings library (search, side, level filters) | `#/openings` | `openings.js`, `/api/openings` | P0 |
| Opening detail: line, ideas, traps, play through, practice | `#/openings/:id` | `/api/openings/:id`, `Board`, `MoveList` | P0 |
| Opening explorer (book continuations) | `#/openings/:id`, `#/analysis` | `/api/openings/lookup` → `continuations` (weight bar) | P1 |
| Opening practice ("play the line from memory") | `#/openings/:id` | compare to `Opening.uci`, bot replies from book | P1 |
| Endgame drills list (by category, goal badge) | `#/endgames` | `endgames.js`, `/api/endgames` | P0 |
| Endgame drill vs engine | `#/endgames/:id` | `/api/bot/move` with strongest bot (or `/api/engine/analyze`), hint + technique list | P0 |
| Game archive / library (filters, search, favourite, tags, notes) | `#/library` | `library.js`, `/api/games`, `PUT /api/games/:id` | P0 |
| PGN import / export | `#/library` | `/api/games/import`, `/api/games/:id/pgn` | P0 |
| Stats / insights (W/L/D, per-bot record, accuracy trend, puzzle rating graph) | `#/profile` | `profile.js`, `/api/stats`, `/api/profile` | P1 |
| Board/piece themes, sounds, coords, legal dots, animation, auto-queen, notation, dark/light | `#/settings` | `settings.js` (page + module), live preview `Board` | P0 |
| Coordinates trainer | `#/learn` (tool card) → page-local mode in `learn.js` | `Board` with `onSquareClick` | P1 |
| Lichess-style "Basics" star stages | `#/learn/basics/...` | `courses.json` steps with tasks | P1 |
| Local 2-player pass-and-play | `#/play` (option) | `Board movableColor:'both'` | P2 |

---

## 3. UX details to copy

### 3.1 Move classifications: colours and symbols
These are chess.com's icon colours. `ui.classificationMeta(cls)` should return exactly these values.
Icons render as a filled circle in the colour with a white glyph. Use them on board badges, in the move list
next to the SAN, on eval-graph dots and in the summary table.

| Class | Colour | Symbol | Shown as / meaning (coach phrasing) |
|---|---|---|---|
| brilliant | `#26c2a3` (teal) | `!!` | "A brilliant sacrifice!" Good piece sacrifice, rare. |
| great | `#5c8bb0` (blue) | `!` | "A great move, the only good one here." |
| best | `#81b64c` (green) | `★` | "That's the best move." |
| excellent | `#96bc4b` (light green) | `👍` | "Excellent, nearly the best." |
| good | `#96af8b` (sage) | `✓` | "A good move." (CONTRACT's symbol list omits it, so use `✓`) |
| book | `#a88865` (brown) | `📖` | "A known opening move." |
| inaccuracy | `#f7c631` (yellow) | `?!` | "An inaccuracy, there was a better move." |
| mistake | `#ffa459` (orange) | `?` | "A mistake." |
| miss | `#ff7769` (salmon) | `✗` | "You missed a chance to punish your opponent." |
| blunder | `#fa412d` (red) | `??` | "A blunder." |
| forced | `#97a0a6` (grey) | `□` | "The only legal move." |

Rules:
- Board badge: about 40% of the square size, positioned at the top-right corner of the **destination** square,
  slightly overflowing the square, with a drop shadow. It pops in (scale 0.6→1, 150ms).
- The played move's from/to squares get a tint of the class colour at ~45% opacity in review mode. The normal
  yellow last-move highlight is used elsewhere.
- Best-move arrow in review is **green** (`#81b64c`, ~0.8 opacity). The played bad move can be shown with the class colour.
- Summary table row order: Brilliant, Great, Best, Excellent, Good, Book, Inaccuracy, Mistake, Miss, Blunder.
  Rows with 0 count are dimmed, not hidden.
- In the move list show the icon only for the notable classes (brilliant, great, inaccuracy, mistake, miss, blunder)
  by default. Showing every icon is noisy. In review mode show all of them.

### 3.2 Eval bar
- Vertical, the same height as the board, ~24px wide (16px on mobile), on the left side of the board.
- White fills from the bottom when the user is white (it flips with board orientation). Colours: white `#f0f0f0`,
  black `#403d39`.
- Map score → fill with win% (`win_percent` / lichess formula `50 + 50*(2/(1+e^(-0.00368208*cp))-1)`), not linear cp,
  so +1 looks meaningful and +8 doesn't overshoot. Clamp the visible fill to [3%, 97%] unless it is mate.
- Mate: the bar is filled fully for the mating side and the label shows `M3`. A mated position shows `1-0` / `0-1` / `½-½`.
- The score label is small bold text (11px) inside the bar at the **winning side's end** (bottom if white is better
  from white's orientation), in the contrasting colour. Format `+1.2`, `-0.4`, `0.0` (ui.formatScore).
- Animate the fill with `transition: height 300ms ease-out`. Throttle updates (engine info ≤10/s already).
- Hover tooltip: "White is better by 1.2 pawns" in plain words for beginners.

### 3.3 Bot selection screen (`#/play`)
Layout (desktop): left = large board preview with the selected bot's avatar and speech bubble greeting
over/above it. Right = side panel:
1. Header "Play vs Bots".
2. Selected bot hero card: big avatar (emoji in a coloured circle), name, Elo, style tag pill, description.
3. Bot grid grouped by `category` headings (Beginner, Intermediate, Advanced, Master, Coach). Tiles show
   avatar + name + Elo. Selected tile has a green ring. Highlight a "Recommended for you" bot from the user's
   record (first bot the user hasn't beaten).
4. Colour chooser: three square buttons (white king, random/half-half, black king). Random is the default.
5. Mode: *Challenge* / *Friendly* (hints, takebacks, eval bar on) / *Custom* expandable toggles. Friendly is the default for beginners.
6. Optional time control chips: No clock (default), 10 min, 5|5, 3|2.
7. Sticky full-width primary button **Play** (green `#81b64c`, bold, big, with a darker bottom border "3D" look as on chess.com).

On mobile the board preview is hidden and the panel is full width.

In game: the bot avatar, name and Elo sit above the board, the user below. The bot's speech bubble appears next
to its avatar, auto-hides after ~4s (clear the timer on unmount!), and the bot "thinks" for `think_ms`
(show three animated dots). The controls bar under the board has these buttons: Hint (bulb), Takeback (undo), Flip, Resign (flag, asks for confirmation),
Offer draw (bots decide by eval).

### 3.4 Hint flow
1. Press **Hint** once: highlight the from-square of the best move (`kind:'hint'`, pulsing soft yellow/green glow).
   The coach says "Look at your knight…" (piece name from SAN).
2. Press again: show the arrow from→to (green) and the SAN in the bubble.
3. Hints used are counted and stored in game notes/tags (`assisted`). In puzzles, a hint fails the rating attempt.
   Same 2-stage behaviour in puzzles, lessons (`task.hint` text first, then arrow) and endgame drills.
4. Clear hint highlights on the next user move.

### 3.5 Puzzle feedback
- On load: set the position to `fen`, then after 500ms animate `moves[0]` (opponent's move). Then show the banner
  "Your turn: find the best move for White" with a small coloured square indicating side.
- Correct move: green square tint on to-square + `✓` badge in `#81b64c`, `correct` sound, then the opponent's reply
  plays automatically after 300–400ms. Last move solved: banner turns green "Solved!" with the rating delta pill
  (`+9` green / `-14` red) and a **Next puzzle** primary button. Auto-advance is optional (setting).
- Accept any mating move as correct when the puzzle's last move delivers mate (alternate mates are valid, as on lichess).
- Wrong move: red tint + `✗` badge (`#fa412d`), `wrong` sound, revert after 600ms. The attempt is recorded as failed
  (only the first try counts) and the user can keep trying or press **Solution** (animates remaining moves).
- Show the puzzle rating, themes (after solving, to avoid spoilers) and an **Analyze** link → `#/analysis?fen=`.

### 3.6 Puzzle Rush
- Start screen: three big mode cards **3 min**, **5 min**, **Survival**, each showing the personal best. Rules line:
  "Solve as many as you can. 3 mistakes and you're out."
- In run: a large timer (counts down; Survival shows elapsed), a large score number, and **three strike slots** that fill red
  `✗` one by one. A results strip of small tiles: green with puzzle rating for solved, red for failed.
- A wrong move = strike + immediately next puzzle (no retries). Difficulty ascends (API returns sorted by rating).
  Pre-fetch more when fewer than 10 remain.
- Ends on 3 strikes or time out. End screen: score, "New personal best!" with confetti if beaten (`POST /api/puzzles/rush`),
  and a grid of all attempted puzzles, each clickable to review in analysis. Buttons **Play again** / **Back**.
- Rush never changes the puzzle rating.
- Timer via one `setInterval` (or rAF) that is cleared on unmount and on game end.

### 3.7 Game Review flow (`#/review/:gameId`)
1. Loading: coach avatar + "Reviewing your game…" + progress bar, so the user waits with something to look at.
2. **Summary card**: coach bubble with `summary`, eval graph, a two-column header (White avatar | Black avatar),
   **Accuracy** big numbers (white tile vs dark tile), **Game Rating** (`estimated_elo`), the classification count table
   (§3.1), the opening name, and phases (Opening/Middlegame/Endgame with the worst icon per phase, P2). Primary button **Start Review**.
3. **Step mode**: board + eval bar left. On the right, the coach bubble: `[icon] **Nf3** is a mistake` + `explanation`.
   Under it the buttons **Best** (toggle the best move arrow + `best_line_san` preview), **Retry** (lets the user try a move
   from `fen_before`: correct = class ≤ excellent → celebratory text), **◀ / ▶** and **Next key moment**.
   Keyboard: ←/→ step, ↑/↓ key moments, `f` flip.
4. Toggle "Show: Me / Opponent / Both" for which side's icons are shown.
5. The eval graph stays visible. Clicking it jumps. Current ply marker = vertical line.

### 3.8 Lesson step flow
- Header: course title › lesson title, a progress bar (`step i / n`), close (X) back to course.
- Two-pane: board left (arrows/highlights from the step, orientation from the step), text card right (markdown-lite)
  with a coach avatar. On mobile the text goes below the board.
- Step without task: **Next** button (and ← Back). Step with task: board becomes interactive, prompt shown in bold,
  a **Hint** button, and Next is disabled until solved. Correct move → green feedback + `success` text, and the scripted reply
  plays automatically. Wrong → red shake + "Not quite, try again" (unlimited retries, no penalty).
- Last step → "Lesson complete!" card with a check animation, `POST /api/progress`, buttons **Next lesson** / **Back to course**.
- Course page shows each lesson with a check if completed and a "Continue" button on the first incomplete lesson.

### 3.9 Openings
- Library cards: mini board of `fen` (static, non-interactive), name, ECO pill, side pill (White/Black repertoire),
  popularity as 1–5 dots, level. Filters: search box, side, level. Group by `family`.
- Detail: board + move list of the line with **▶ Play through** (auto-steps at 800ms with a pause button; clear the timer!),
  ideas as bullet list with a lightbulb, traps as warning-styled cards, and an **Explorer** panel: book continuations
  with SAN, opening name and a weight bar. **Practice** mode: user plays their side from memory. A wrong move gives
  "That's not the main line. The book move was **Nf3**" and lets them retry. After 3 clean runs mark it "learned" (P2).

### 3.10 Endgame drills
- List grouped by category with a goal badge: green "WIN" / blue "DRAW", level, and a check for completed (local progress).
- Drill page: goal banner ("White to play and win"), the technique checklist in a side card, Hint (shows `hint`, then the engine's best move arrow),
  Reset position, and the strongest engine as opponent. Success detection: checkmate (win), or for draw goals,
  draw by rule or surviving N moves (e.g. 30 plies) without losing. Failure: opponent mates or eval swings past threshold → "Try again".

### 3.11 General look & feel (chess.com cues)
- Dark charcoal background (`#312e2b` page, `#262421` panels, `#3c3a37` raised), off-white text, green primary
  `#81b64c` with hover `#a3d160`, buttons with 4px bottom "depth" border and 6–8px radius.
- Default board: green theme light `#eeeed2` / dark `#769656`. Last-move highlight yellow `rgba(255,255,51,.5)`.
  Selected square is the same yellow. Legal-move dots are 30%-black circles (rings on capture squares).
  Check = radial red glow on the king square.
- Speech bubbles: white card, dark text, small tail toward the avatar, 14–15px font, subtle drop shadow.
- Sounds: distinct move/capture/check/castle/game-end/correct/wrong, all mutable.
- Beginner friendliness: always say whose turn it is, never show raw engine output without a plain-language line,
  use tooltips on every icon button, keep primary actions big and green.
- Responsive: board takes `min(100vw - 32px, 100vh - header)` on mobile, and side panels stack below.

---

## 4. Out of scope / later
Online multiplayer, tournaments, clubs, leaderboards across users, video lessons, anti-cheat, Puzzle Battle vs humans,
Daily (correspondence) games, and a game database with masters' stats (needs a large dataset).
