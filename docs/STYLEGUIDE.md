# GrandMentor Style Guide

The design system lives in `web/css/app.css` (tokens, shell, components, layouts, utilities)
and `web/js/ui.js` (DOM helper, icons, toast, modal, formatters). Page- and board-specific CSS
goes in `web/css/pages.css` / `web/css/board.css` and **must use the tokens below**, never raw hex
values, so dark and light themes both work.

Principles: dark-first, calm surfaces, one strong accent (green `--primary`), big friendly hit
targets (≥ 40px), short plain-language copy for beginners, and motion that is quick and subtle.

---

## 1. Tokens (CSS custom properties on `:root`)

Dark theme is the default. Light theme: `<html data-theme="light">` (set by `settings.js`).

### Surfaces & text
| Token | Use |
|---|---|
| `--bg` | page background |
| `--bg-elev` | sidebar, bottom bar |
| `--surface` | cards, panels, modals |
| `--surface-2` | inputs, nested blocks |
| `--surface-3` | chips, hover on surface-2, secondary buttons |
| `--surface-hover` | row/list hover |
| `--surface-sunken` | wells, toolbars, code blocks |
| `--overlay` | modal backdrop |
| `--border`, `--border-strong`, `--divider` | borders (divider = hairline) |
| `--text`, `--text-muted`, `--text-subtle`, `--text-inverse` | text hierarchy |
| `--link` | links |

### Brand & semantic
`--primary` (#81b64c) `--primary-hover` `--primary-active` `--primary-shadow` (3D button edge)
`--primary-soft` (tinted bg) `--primary-contrast` · `--accent` `--accent-soft` ·
`--success` `--success-soft` · `--warning` `--warning-soft` · `--danger` `--danger-hover`
`--danger-shadow` `--danger-soft` · `--info` `--info-soft` · `--gold`.

**Contrast rules (WCAG AA).** The semantic colours above are for fills, borders, icons and charts. For **text** use the
text-safe variants `--primary-text`, `--success-text`, `--danger-text`, `--info-text`, `--accent-text`,
`--warning-text` (≥ 4.5:1 on cards and tinted chips, in both themes). For a **fill behind white text** use
`--primary-fill` (+ `-hover`, `-edge`) and `--danger-fill` (+ `-hover`): plain `--primary` / `--danger` are too light for
white text. `--text-subtle` is the lowest-contrast text allowed (≥ 4.5:1 on `--surface`, `--surface-2`, `--surface-3`).
`--focus` is the focus-ring colour; `--ring` (2px gap + 2px ring) is the standard focus style.

### Move classifications
| Token | Color | Symbol (`ui.classificationMeta`) |
|---|---|---|
| `--cls-brilliant` | #26c2a3 teal | `!!` |
| `--cls-great` | #6f9fc4 blue | `!` |
| `--cls-best` | #81b64c green | `★` |
| `--cls-excellent` | #96bc4b | `👍` |
| `--cls-good` | #96af8b | `✓` |
| `--cls-book` | #a88865 | `📖` |
| `--cls-inaccuracy` | #f7c631 | `?!` |
| `--cls-mistake` | #ffa459 | `?` |
| `--cls-miss` | #ff7769 | `✗` |
| `--cls-blunder` | #ff6a58 | `??` |
| `--cls-forced` | #96af8b | `→` |

Any element with `data-cls="<classification>"` gets a local `--cls` variable set to its color,
so you can write `color: var(--cls)` / `background: var(--cls)` in your own CSS.
The light theme darkens every `--cls-*` (≥ 4.5:1 as text on white) and sets `--cls-fg: #fff` (badge symbol colour; dark
`#111417` in the dark theme). `<html data-cls-palette="cb">` (setting **Colour-blind friendly colours**) swaps in an
Okabe–Ito based palette: good moves in blues/teal, bad moves in yellow → orange → purple → vermillion. **Never rely on
the colour alone**: always show the symbol (`classificationBadge()`, `.gm-ml-dot`) or the label next to it.

### Board & eval (for `board.css`, `evalbar.js`, etc.)
`--board-light`, `--board-dark` (set inline on `<html>` from the `boardTheme` setting),
`--board-coord-light` (coordinate text drawn on light squares), `--board-coord-dark`,
`--board-lastmove`, `--board-selected`, `--board-check` (a radial-gradient for the king square),
`--board-dot` (legal-move dot), `--board-hint`, `--board-good`, `--board-bad`,
`--arrow-green|red|blue|yellow`, `--anim-ms` (from `animationMs` setting),
`--eval-white`, `--eval-black`, `--eval-text-on-white`, `--eval-text-on-black`.

`<html>` also carries `data-board-theme`, `data-piece-set` and `data-coords="on|off"`.
Piece image URL: `settings.pieceUrl('wK')` → `/img/pieces/<set>/wK.svg`.

### Typography
`--font-sans` (Inter + system fallback), `--font-mono`.
Sizes: `--fs-xs` 12 · `--fs-sm` 13 · `--fs-md` 15 (body) · `--fs-lg` 17 · `--fs-xl` 20 ·
`--fs-2xl` 26 · `--fs-3xl` 34 · `--fs-4xl` 44. Weights: `--fw-regular|medium|semibold|bold|black`.
Line heights: `--lh-tight`, `--lh`.

### Spacing, radii, shadows, z-index, motion
- Spacing (4px scale): `--sp-1` 4 · `--sp-2` 8 · `--sp-3` 12 · `--sp-4` 16 · `--sp-5` 20 · `--sp-6` 24 · `--sp-7` 32 · `--sp-8` 40 · `--sp-9` 48 · `--sp-10` 64
- Radii: `--r-xs` 4 · `--r-sm` 6 · `--r-md` 10 · `--r-lg` 14 · `--r-xl` 20 · `--r-full`
- Shadows: `--shadow-xs|sm|md|lg|xl`, focus ring `--ring`
- Z-index: `--z-base` 1 · `--z-board-overlay` 10 · `--z-sticky` 20 · `--z-header` 40 · `--z-sidebar` 50 · `--z-dropdown` 100 · `--z-sheet` 900 · `--z-modal` 1000 · `--z-toast` 1100 · `--z-tooltip` 1200
- Motion: `--ease`, `--ease-out`, `--ease-spring`, `--dur-fast` 120ms, `--dur` 200ms, `--dur-slow` 320ms. `prefers-reduced-motion` is honoured globally.

### Layout tokens
`--sidebar-w`, `--sidebar-w-collapsed`, `--bottombar-h`, `--content-max` (1240px), `--page-pad`,
`--panel-w` (game side panel), `--evalbar-w`, `--board-gap`, `--board-chrome` (vertical space
reserved around the board), `--board-max`. Breakpoints: **1100px** (tablet), **860px** (mobile: sidebar
→ bottom tab bar), **560px** (small phone).

---

## 2. App shell & pages

`index.html` provides `#sidebar`, `#view`, `#bottombar`, `#more-sheet`, `#toasts`; `app.js` renders
navigation and mounts pages into a fresh empty `<div class="page-host">` inside `#view`.

**Page module** (`web/js/pages/<name>.js`):
```js
import { h, icon, pageHeader, disposables } from '../ui.js';
export const title = 'Puzzles';
export async function mount(root, { params, query, path }) {
  const bag = disposables();
  const page = h('div', { class: 'page' }, pageHeader({ title: 'Puzzles', icon: 'puzzle', subtitle: 'Train your tactics' }));
  root.appendChild(page);
  bag.on(window, 'keydown', onKey);
  return bag.dispose;           // MUST remove every listener / timer / socket / component
}
```

### Routes → page module & params
| Hash | Module | `params` |
|---|---|---|
| `#/` | home | – |
| `#/play`, `#/play/:botId` | play | `botId?` |
| `#/analysis?fen=…|?game=:id|?pgn=…` | analysis | (use `query`) |
| `#/review/:gameId` | review | `gameId` |
| `#/puzzles` | puzzles | – |
| `#/puzzles/rush` | puzzles | `mode: 'rush'` |
| `#/puzzles/daily` | puzzles | `mode: 'daily'` |
| `#/learn`, `#/learn/:courseId` | learn | `courseId?` |
| `#/learn/:courseId/:lessonId` | lesson | `courseId, lessonId` |
| `#/openings`, `#/openings/:id` | openings | `id?` |
| `#/endgames`, `#/endgames/:id` | endgames | `id?` |
| `#/library` · `#/profile` · `#/settings` | library · profile · settings | – |

Params are URI-decoded; `query` is a plain object from `URLSearchParams` (encode values with
`encodeURIComponent` — note `+` must be `%2B`). Navigate with plain links (`href="#/play/martin"`)
or `location.hash = '#/review/12'`; `app.js` also exports `navigate(path, {replace})`.

The router: calls the previous cleanup first, closes open modals, sets `document.title`
(`title` export may be a string or `(params, query) => string`), highlights the nav, fades the page
in, and shows a friendly error card if `import`/`mount` throws. Collapsing the sidebar dispatches a
`resize` event so boards can re-measure.

### Page containers
```html
<div class="page">…</div>            <!-- centered, max-width 1240px, padded -->
<div class="page page-wide">…</div>  <!-- full width -->
```

### Page header
```html
<header class="page-header">
  <div class="page-header-main">
    <div class="page-header-icon"><!-- icon('puzzle') --></div>
    <div><h1 class="page-title">Puzzles</h1><p class="page-subtitle">Train your tactics</p></div>
  </div>
  <div class="page-actions"><button class="btn btn-primary">Start</button></div>
</header>
```
JS: `pageHeader({ title, subtitle, icon, actions: [nodes], breadcrumbs: [{label, href}] })`.
Breadcrumbs markup: `<nav class="breadcrumbs"><a href="#/learn">Learn</a> › <span>Course</span></nav>`.
Section headings: `<h2 class="section-title">Popular lines <a href="…">See all</a></h2>`.

---

## 3. Components

### Buttons
```html
<button class="btn">Default</button>
<button class="btn btn-primary">Play</button>      <!-- green 3D button, main CTA -->
<button class="btn btn-secondary">Secondary</button>
<button class="btn btn-ghost">Ghost</button>
<button class="btn btn-outline">Outline</button>
<button class="btn btn-danger">Resign</button>
<button class="btn btn-primary btn-sm|btn-lg|btn-xl">Sizes</button>
<button class="btn btn-primary btn-block">Full width</button>
<button class="btn btn-ghost btn-icon" aria-label="Flip board"><!-- icon('flip') --></button>
<button class="btn btn-primary loading">Saving</button>   <!-- spinner, disables clicks -->
<div class="btn-group">…</div>
<div class="toolbar"><button class="btn btn-ghost btn-icon">…</button>…</div>  <!-- board nav controls -->
```
Icons go first inside the button: `h('button', {class:'btn btn-primary', html: icon('play') + '<span>Play</span>'})`.

### Cards & panels
```html
<div class="card">
  <div class="card-header"><div class="card-title"><!-- icon --> Daily puzzle</div><span class="badge">New</span></div>
  <div class="card-body">…</div>
  <div class="card-footer"><button class="btn btn-primary">Solve</button></div>
</div>
<a class="card card-link" href="#/learn/basics"><div class="card-media">♞</div>…</a>  <!-- hover lift -->
<div class="card card-sm">compact</div>   <div class="card card-flush">no padding (lists)</div>
<div class="card card-feature">green glow corner</div>   <div class="card card-highlight">selected</div>
```
Panels (side columns, scrollable bodies):
```html
<div class="panel grow">
  <div class="panel-header"><!-- icon --> Moves</div>
  <div class="panel-body">…scrolls…</div>
  <div class="panel-footer">…</div>
</div>
```
Stat tile: `<div class="stat"><div class="stat-label">Rating</div><div class="stat-value">1234</div><div class="stat-delta up">+12</div></div>`.

### Tabs & segmented control
```html
<div class="tabs" role="tablist">
  <button class="tab active" role="tab" aria-selected="true">Moves</button>
  <button class="tab" role="tab">Mentor</button>
</div>
<div class="tab-panel">…</div>

<div class="segmented"><button class="active">White</button><button>Random</button><button>Black</button></div>
<div class="segmented block">…</div>   <!-- full width, equal segments -->
```

### Badges, pills, chips, classification badges
```html
<span class="badge">Default</span> <span class="badge badge-primary">Win</span>
<span class="badge badge-info|badge-warning|badge-danger|badge-gold|badge-solid">…</span>
<span class="badge badge-lg">Large</span>
<span class="badge level-beginner|level-intermediate|level-advanced|level-master">Beginner</span>
<div class="chip-row"><button class="chip active">Forks</button><button class="chip">Pins</button></div>
<span class="cls-badge" data-cls="blunder">??</span>  <span class="cls-badge lg" data-cls="brilliant">!!</span>
<span class="cls-text" data-cls="mistake">Mistake</span>
```
JS: `classificationBadge('best', {large})`, `classificationMeta('best')` → `{key, label, color, cssVar, symbol, description}`.

### Forms
```html
<div class="field">
  <label class="label" for="q">Search</label>
  <div class="input-group"><!-- icon('search') --><input id="q" class="input" placeholder="Sicilian…"></div>
  <div class="help">Name or ECO code</div>
</div>
<select class="select">…</select>   <textarea class="textarea mono">PGN…</textarea>
<input class="input input-sm">     <div class="form-row"><div class="field">…</div><div class="field">…</div></div>
<label class="switch"><input type="checkbox" checked><span class="switch-track"></span> Sounds</label>
<label class="checkbox"><input type="checkbox"> Favorites only</label>
<input type="range" class="range" min="0" max="500">
<div class="setting-row"><div class="setting-row-text"><div class="setting-row-title">Show coordinates</div>
  <div class="setting-row-desc">Letters and numbers on the board edge</div></div><label class="switch">…</label></div>
```
Error state: add `.error` to `.field`.

### Progress, spinner, skeleton
```html
<div class="progress"><div class="progress-bar" style="width:40%"></div></div>
<div class="progress progress-sm|progress-lg progress-info|progress-warning|progress-danger progress-striped progress-indeterminate">…</div>
<div class="progress-ring" style="--value:72">72%</div>
<div class="spinner"></div> <div class="spinner spinner-lg"></div>
<div class="loading-center"><div class="spinner spinner-lg"></div>Loading…</div>
<div class="skeleton skeleton-title"></div><div class="skeleton skeleton-text"></div>
<div class="skeleton skeleton-card"></div><div class="skeleton skeleton-avatar"></div><div class="skeleton skeleton-board"></div>
```
JS: `loadingBlock('Loading puzzles…')`, `skeleton('card'|'list'|'text', n)`.

### Avatar
```html
<div class="avatar">🤖</div>   <div class="avatar avatar-xs|avatar-sm|avatar-lg|avatar-xl avatar-round"><img src="…" alt=""></div>
```

### Lists
```html
<div class="card card-flush list">
  <a class="list-row" href="#/review/3">
    <span class="result result-win">W</span>
    <div class="list-row-main"><div class="list-row-title">You vs Martin</div><div class="list-row-sub">Italian Game · 34 moves</div></div>
    <div class="list-row-meta">2 h ago</div>
    <!-- icon('chevron-right') -->
  </a>
</div>
```
`.list-row.clickable`, `.list-row.active`, `button.list-row` also supported. Result markers:
`.result-win`, `.result-loss`, `.result-draw`.

### Empty states, callouts, mentor bubbles, hero
```html
<div class="empty-state"><div class="empty-state-icon"><!-- icon --></div>
  <div class="empty-state-title">No games yet</div><p>Play a bot and your games appear here.</p>
  <a class="btn btn-primary" href="#/play">Play now</a></div>
<div class="callout callout-success|callout-warning|callout-danger"><!-- icon --><div>Text</div></div>
<div class="mentor-row"><div class="avatar avatar-sm">🎓</div><div class="bubble bubble-mentor">Nice fork!</div></div>
<div class="bubble bubble-user">Why not Nf3?</div>
<section class="hero"><div><h1 class="hero-title">Learn chess <em>the fun way</em></h1>
  <p class="hero-sub">…</p><div class="hero-actions">…</div></div><div>…</div></section>
```
JS: `emptyState({ icon, emoji, title, text, action: {label, href|onClick, icon, kind} })`.

### Tooltip (CSS only)
```html
<button class="btn btn-icon" data-tooltip="Flip board" aria-label="Flip board">…</button>
<span data-tooltip="Right side" data-tooltip-pos="bottom|right">…</span>
```
Hidden on touch devices — never put essential info only in a tooltip.

### Modal (JS)
```js
const m = modal({
  title: 'Game over', size: 'lg' /* optional */, dismissible: true,
  body: nodeOrText,                        // strings are escaped text
  actions: [
    { label: 'Rematch', kind: 'ghost', icon: 'refresh', onClick: () => rematch() },
    { label: 'Game Review', kind: 'primary', onClick: async (close) => { … /* return false to keep open */ } },
  ],
  onClose: () => {},
});
m.close();   // m.el (dialog), m.body (body element)
const ok = await confirmDialog({ title: 'Delete game?', message: '…', confirmLabel: 'Delete', danger: true });
```
Markup (if hand-built): `.modal-backdrop > .modal(.modal-lg) > .modal-header(.modal-title) + .modal-body + .modal-footer`.
Result banner inside a modal: `.result-hero > .result-hero-title + .result-hero-sub`.
Esc / backdrop close, focus trap and focus restore are built in. The router closes modals on navigation.

### Toast (JS)
`toast('Game saved', 'success')` — kinds `info | success | warning | error`; options `{duration}` (ms, 0 = sticky).
Returns `dismiss()`. Max 4 visible.

### Misc
`<kbd>←</kbd>`, `.divider`, `.dot-sep` (inline ·), `.md` (wrapper for `mdLite()` HTML), `.fen` (mono FEN text),
animations `.pulse`, `.pop-in`, `.shake` (wrong move), `.page-enter`.

---

## 4. Layout classes

### Grids & stacks
`.grid` · `.grid-2` · `.grid-3` · `.grid-4` (collapse responsively) · `.grid-auto` (auto-fill, min 240px;
`.grid-auto-sm` 180px, `.grid-auto-lg` 320px, or set `style="--min:200px"`) · `.split` (main + 340px aside) ·
`.stack` / `.stack-sm` / `.stack-lg` (vertical gaps 12/8/24) · `.row` / `.row-sm` (horizontal, centered) ·
`.row-wrap` · `.spacer` (flex:1) · `.between` · `.center`.

### Game layout (play, analysis, review, puzzles, lessons, endgames)
```html
<div class="game-layout">            <!-- add .no-eval to hide the eval column -->
  <div class="game-main">
    <div class="player-bar">
      <div class="avatar">🤖</div>
      <div><div class="player-name">Martin <span class="player-rating">(250)</span></div>
           <div class="player-captures">♟♟</div></div>
      <div class="clock-slot"><!-- ChessClock --></div>
    </div>
    <div class="board-row">
      <div class="evalbar-slot"><!-- new EvalBar(el) --></div>
      <div class="board-slot"><!-- new Board(el) --></div>
    </div>
    <div class="player-bar">…</div>
  </div>
  <aside class="game-panel">
    <div class="panel grow">…move list / mentor…</div>
    <div class="toolbar">…first / prev / next / last / flip…</div>
  </aside>
</div>
```
`.game-layout` computes `--board-size = clamp(260px, min(100dvh − --board-chrome, available width), --board-max)`;
`.board-slot` and `.evalbar-slot` use it so the board is always square and fits the viewport.
`.game-panel` matches board height on desktop and stacks under the board on mobile.
`.board-slot` is `position:relative` and a size container (`cqw` units work inside).
Tweak per page: `style="--board-chrome:96px"` (no player bars) or `--panel-w:420px`.
Engine lines: `<div class="engine-line"><span class="engine-score [neg]">+0.45</span><span class="engine-moves">e4 e5 Nf3</span></div>`.

---

## 5. Utilities
`.muted` `.subtle` `.text-primary` `.text-danger` `.text-warning` `.text-xs|sm|lg|xl` `.text-center`
`.bold` `.semibold` `.mono` `.tabular` `.truncate` `.nowrap` `.w-full`
`.mt-1|2|3|4|6|8` `.mb-1|2|3|4|6` `.gap-1|2|4|6` `.p-0|3|4`
`.hide-mobile` `.show-mobile` `.sr-only` `.icon` `.icon-sm` `.icon-lg`.

---

## 6. `ui.js` API reference

| Export | Notes |
|---|---|
| `h(tag, attrs, ...children)` | `class` (string/array), `style` (string/object, supports `--vars`), `dataset`, `html` (trusted), `onClick` etc., `ref`, booleans. SVG tags supported. |
| `htmlToNode(html)`, `clear(el)` | |
| `escapeHtml(s)` | always escape user/server text you put into `innerHTML` |
| `icon(name, {size, cls, label, strokeWidth})` → SVG string; `iconNode(...)` → Node; `ICON_NAMES` | |
| `brandMark(size)` | logo SVG |
| `formatScore(score, {pov, digits})` | `{cp:123}`→`+1.2`, `{mate:3}`→`M3`, `{mate:-2}`→`-M2` |
| `scoreToNumber(score, clamp=10)`, `winPercent(score)` | white POV, for graphs / eval bar |
| `classificationMeta(cls)`, `classificationBadge(cls)`, `CLASSIFICATIONS` | |
| `formatSan(san, notation)` | `'figurine'` → ♘f3 |
| `formatClock(ms)`, `formatRelative(date)`, `formatDate(date)` | |
| `mdLite(text)` | safe HTML for `**bold**`, `*italic*`, `` `code` ``, line breaks |
| `debounce(fn, ms)` (`.cancel()`), `disposables()` | cleanup bag: `on`, `timeout`, `interval`, `raf`, `add`, `dispose` |
| `copyText(text)` | clipboard + toast |
| `toast`, `modal`, `confirmDialog`, `closeAllModals` | |
| `pageHeader`, `emptyState`, `loadingBlock`, `skeleton`, `comingSoon` | ready-made blocks |

### Icon names (`icon(name)`)
Navigation: `home play puzzle learn openings endgames analysis library profile settings more menu`
Pieces: `knight king queen rook bishop pawn crown board`
Controls: `chevron-left chevron-right chevron-up chevron-down first last arrow-left arrow-right flip undo redo refresh play-circle pause`
Actions: `close x check plus minus search filter edit trash copy download upload share external save link eye eye-off lock`
Status: `info alert check-circle x-circle help hint star star-filled heart trophy medal target flag handshake bolt fire clock timer calendar chart sparkles shield swords`
People/chat: `robot mentor chat send book folder tag user users`
Misc: `sun moon volume volume-off sidebar grid list palette keyboard wifi wifi-off`

---

## 7. `api.js` & `settings.js` quick reference
```js
import { api, qs, isAbort, ApiError, EngineClient } from '../api.js';
const games = await api.get('/api/games' + qs({ search, limit: 50 }), { signal });  // throws ApiError(message, status)
const engine = new EngineClient();
engine.analyze(fen, { multipv: 3, movetime_ms: 4000 }, (info, done) => {/* info.lines[0].score */}, (err) => {});
engine.stop(); engine.close();   // close() in cleanup!

import { getSettings, getSetting, setSetting, onSettingsChange, resetSettings, BOARD_THEMES, PIECE_SETS, DEFAULTS, pieceUrl } from '../settings.js';
const off = onSettingsChange((s, key, value) => {...});   // call off() in cleanup
```
Settings keys: `boardTheme` (green|brown|blue|purple|gray), `pieceSet` (cburnett|merida|chessnut; a stored `alpha` is migrated to `chessnut`), `sounds`,
`showCoords`, `showLegal`, `animationMs` (0–1000), `showEvalBar`, `autoQueen`, `theme` (dark|light),
`moveNotation` (san|figurine), plus shell-only `sidebarCollapsed`. Invalid values are rejected.

## 8. Writing for beginners
- Prefer verbs and outcomes: "Play a bot", "Review this game", "Try again".
- Explain chess terms the first time (tooltip or one-line help text).
- Celebrate progress (toasts, badges) and keep error copy friendly and actionable.

## 9. Accessibility

- **Keyboard.** Everything clickable is a `<button>` or `<a href>` (or has `tabindex="0"` + a role + Enter/Space
  handling). Never remove the focus style; `:focus-visible` gets `--ring` everywhere (high-contrast mode adds a 3px
  outline). The board is one tab stop with arrow keys inside (see CONTRACT "Accessibility"). Modals trap focus and give it
  back on close. The closed mobile "More" sheet is `inert`. The skip link (`.skip-link`) appears on focus and moves focus
  to the page's `<h1>`.
- **Names.** Icon-only buttons need `aria-label` (and usually `title`/`data-tooltip`). Inputs need a `<label>` or
  `aria-label`. Charts are `role="img"` with an `aria-label` that states the numbers; the eval graph is a `slider` with
  `aria-valuetext` ("12. Nf3, +0.4, Mistake").
- **Live regions.** Use `announce()` from `components/announcer.js` instead of adding new `aria-live` regions. If a
  status element must be live, only rewrite it when its text changes (see the puzzle status) so it doesn't repeat.
- **Touch targets.** Buttons, chips and form controls are at least 40×40 px on phones (≤ 860px wide); `.btn-sm`,
  `.chip`, `.segmented` buttons, move-list moves grow automatically.
- **Motion.** Don't check `prefers-reduced-motion` yourself: use `reducedMotion()` / `scrollBehavior()` from
  `settings.js`, and in CSS `:root[data-motion="reduce"]`. Confetti (`.lrn-confetti`, `.pz-confetti`, `.dp-confetti`) is
  hidden and every animation/transition is cut to 1 ms when motion is reduced; the board sets its slide time to 0.
- **High contrast.** `:root[data-contrast="high"]` (dark or light) raises text to AAA (`--text`, `--text-muted`,
  `--text-subtle` ≥ 7:1), strengthens borders and dividers, outlines active segmented/nav items and underlines links.
  Build new components from the tokens and they follow automatically.
- **Board theme `contrast`.** Near-white / slate-blue squares (4.6:1), stronger last-move and selection tints, darker
  legal-move dots, bold coordinates.
- **Text size.** `uiScale` sets `<html>` font-size to 100/115/130 %; size text in `rem` tokens (`--fs-*`), not px, so it
  scales.
- **Automated checks.** `tools/qa/sweep.mjs` reports `a11y-name`, `a11y-alt`, `a11y-hidden-focus`, `a11y-dup-id`,
  `contrast`, `touch-target` and `keyboard` issues (see `tools/qa/README.md`). Run it with
  `QA_SETTINGS='{"theme":"light"}'` and `QA_SETTINGS='{"highContrast":true}'` too when you change colours.
