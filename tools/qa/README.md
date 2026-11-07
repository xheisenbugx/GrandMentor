# Quality gate (`tools/qa/`)

Automated versions of the manual checks in the Definition of done. Plain Node 22 scripts, no npm
dependencies. The same commands run in GitHub Actions (`.github/workflows/ci.yml`) on every pull
request to `dev`/`main` and every push to `dev`.

| Script | What it checks | Needs |
|---|---|---|
| `check-syntax.mjs` | Every `.js`/`.mjs` file in `web/`, `tools/` and `tests/` parses | Node |
| `tests/js/*.test.mjs` | Unit tests for pure frontend modules (`node --test`) | Node |
| `check-i18n.mjs` | Locale catalogs and `t()` keys used in code | Node |
| `sweep.mjs` | Every route × language × viewport in headless Chrome | Rust (it builds the server), Chrome |
| `bench-gate.mjs` | Engine speed and tactical strength against `bench-baseline.json` | Rust (it builds `gm-bench-gate`) |
| `cargo deny check` | RustSec advisories, licenses, crate sources, duplicate versions (`deny.toml`) | `cargo install cargo-deny --locked` |

## Run it before opening a PR

```bash
cargo build --release --workspace
cargo deny check                      # a few seconds (fetches the RustSec database)
node tools/qa/check-syntax.mjs        # a few seconds
node --test 'tests/js/**/*.test.mjs'  # under a second
node tools/qa/check-i18n.mjs          # a few seconds
node tools/qa/bench-gate.mjs          # about 5 s plus the build
node tools/qa/sweep.mjs               # several minutes; builds and starts its own server on a free port and a throwaway database
```

`sweep.mjs` and `bench-gate.mjs` run `cargo build --release` for their binary first (a no-op when it is up to date),
so they never test a stale build. Set `GM_BIN` / `BENCH_BIN` to use a prebuilt binary as is (CI does this).

Each script exits with `0` when clean and `1` when it found problems. `sweep.mjs` exits with `2` when it
could not run at all (no Chrome, the server did not start or exited, `QA_PORT` already in use).

## `check-syntax.mjs`

ES modules are checked with `node --input-type=module --check` (plain `node --check file.js` does not reliably
treat these files as modules). Two files are classic scripts and are parsed as such, so an `import` in them fails:
`web/sw.js` (the service worker is registered without `type: 'module'`) and `tools/qa/page-checks.js`
(evaluated inside the page by the sweep).

## JS unit tests (`tests/js/`)

`node --test 'tests/js/**/*.test.mjs'` runs zero-dependency tests for pure modules: `i18n.js` (lookup, `{placeholders}`,
plural forms, language switching), the router's `parseHash`/`matchRoute` and the `ROUTES` table in `app.js`,
the formatting helpers in `ui.js`/`api.js` and `sound-catalog.js`. `tests/js/dom-shim.mjs` provides the few browser
globals these modules touch at import time (`localStorage`, `<html>`, `window`, `location`) and keeps
`document.readyState` at `"loading"` so `app.js` never boots. Add a `*.test.mjs` file next to the others for new
pure helpers; anything that needs real DOM belongs in the sweep.

## `check-i18n.mjs`

Loads `web/locales/<lang>/index.js` for every language in `web/js/languages.js` and compares it with English.

**Errors:**
- missing or extra keys
- `{placeholders}` that differ from English
- string, list and plural mismatches
- plural objects without `other`
- empty strings
- locale files that `index.js` does not import
- keys used in `web/js` that English does not define. This covers `t('a.b')` and `` t(`a.b`) `` (also `tHtml`, `tOr`). Dynamic keys such as `` t(`play.modes.${id}.label`) `` or `t('nav.' + k)` are checked by prefix.
- a raw `${t(...)}` inside an HTML template literal (one that contains a tag, or is the value of `html:` /
  `innerHTML`). Translations are text: wrap them in `escapeHtml()`, or use `labelHtml(text)` /
  `iconLabel(icon, text)` from `web/js/ui.js`. A message that intentionally contains markup (`<strong>`)
  goes through `tHtml(key, params)`, which escapes the parameters.

**Warnings** (`--verbose` lists them):
- list messages whose length differs from English
- values identical to English, which might be untranslated

`--json` prints a machine-readable report.

## `sweep.mjs`

1. Reads the `ROUTES` table from `web/js/app.js` with a regex, so new pages are picked up with no extra setup.
2. Fills route parameters with real ids from the API:
   - bots, courses and lessons, openings and endgames
   - a game, seeded through the API if the database has none
   - any other `/x/:id` route gets the first item of `GET /api/x`
3. Skips a route and lists it in the report when no data exists for it. **A skipped route fails the sweep**
   (it was never tested) unless `allowlist.json` has a `{ "check": "skipped", "route": "<pattern>", "reason": "…" }` entry.
4. Visits each page in headless Chrome, once per language and per viewport (desktop 1440×900, mobile 390×844 with touch).

Each page is checked for:

| Check | Meaning |
|---|---|
| `console`, `exception` | `console.error`, uncaught exceptions, browser error log entries |
| `http` | same-origin requests answered with 4xx/5xx, or failing at network level (aborted requests are ignored) |
| `timeout` | the network did not go idle within 12 s (the report lists the pending requests) |
| `language`, `empty` | `<html lang>` is not the selected language; the page rendered nothing |
| `i18n-key`, `i18n-missing` | `[i18n] unknown key` / `[i18n] missing … using English` messages from `i18n.js` |
| `raw-key` | visible text, `placeholder`/`aria-label`/`title`/`alt` or the tab title contains a real i18n key such as `play.hint` |
| `junk-text` | visible text or an accessible attribute shows a leaked JS value: `null`, `undefined`, `NaN` or `[object Object]` (usually an optional child passed to native `append()`/`replaceChildren()`) |
| `big-icon` | an `icon()` SVG renders larger than 64 px because no CSS rule sizes it, so it stretches to fill its container |
| `english` | non-English pages showing an English catalog phrase (≥ 14 characters, not shared with the target catalog) |
| `hscroll` | the page scrolls horizontally |
| `overflow` | a visible element sticks out of the viewport. Page-level `overflow-x: hidden/clip` does not hide it, but real scroll containers do |
| `layer-over-board` | a speech bubble, toast, popover or tooltip covers a `.gm-board` |
| `button-overlap` | two buttons overlap. The check skips nested buttons and buttons in different fixed or sticky layers |
| `a11y-name` | a visible button, link, form control or ARIA widget without an accessible name (aria-label, labelledby, `<label>`, text, image alt or title) |
| `a11y-alt` | an `<img>` without `alt`, or a `role="img"` element without a label |
| `a11y-hidden-focus` | a focusable element inside `aria-hidden="true"` |
| `a11y-dup-id` | the same `id` used more than once |
| `contrast` | text below 4.5:1 (3:1 for large text) against the first opaque background up the tree; text over images/gradients, faded (opacity < 1) or disabled content, the board and the eval bar are skipped |
| `touch-target` | on the mobile viewport, a button, chip or form control smaller than 40×40 px (inline text links are exempt) |
| `keyboard` | Tab smoke test (`QA_KBD_STEPS`, default 20 stops from the top): focus must move, land on a visible element outside `aria-hidden`, and show a focus indicator |

Output goes to `QA_OUT` (default `target/qa-sweep/`):
- `report.json`
- `report.md`
- `server.log`
- `shots/<lang>-<viewport>-<route>.png` for pages with issues

| Variable | Default | |
|---|---|---|
| `BASE` | – | test an already running server instead of starting one. It seeds a game when the database has none, so point it at a throwaway database |
| `GM_BIN` | – | prebuilt server binary, used as is. Unset: the sweep runs `cargo build --release -p gm-server --bin grandmentor` first |
| `QA_PORT` | free port | port of the server the sweep starts. Unset: the OS picks a free port. Set: the sweep refuses to start when the port is already taken, so it never tests (and seeds) another server |
| `CDP_PORT` | free port | Chrome DevTools port. Unset: the OS picks a free one, so parallel sweeps never drive each other's Chrome |
| `CHROME`, `CHROME_ARGS` | auto | Chrome executable and extra flags. Detection order: macOS app, then `google-chrome`, then `chromium`. On Linux `--no-sandbox --disable-dev-shm-usage` is the default |
| `QA_LANGS` | all | e.g. `en` or `en,es` |
| `QA_VIEWPORTS` | `desktop,mobile` | |
| `QA_ROUTES` | all | regex over route patterns, e.g. `'^/(play\|review)'` |
| `QA_SAMPLE` | `1` | `N` > 1: non-English languages visit only every N-th route (rotating), to save CI time |
| `QA_SHOTS` | `failures` | `all` or `none` |
| `QA_SETTLE_MS` | `500` | extra wait after the network goes idle |
| `QA_SETTINGS` | – | JSON merged into the app settings before the run, e.g. `'{"theme":"light"}'`, `'{"highContrast":true,"cbPalette":true}'` |
| `QA_KEYBOARD`, `QA_KBD_STEPS` | on, `20` | `QA_KEYBOARD=0` skips the Tab smoke test |
| `QA_INJECT` | – | JS run in every page, to test the sweep itself: `QA_INJECT="console.error('x')"` must fail every page |

## `bench-gate.mjs`

`gm-bench-gate` (built with `cargo build --release -p gm-engine --bin gm-bench-gate` unless `BENCH_BIN` points at a
prebuilt binary) searches reference positions to a fixed depth.
Fresh tables make the node counts deterministic. It also searches six tactical positions and records
the expected best moves: mates in 1 and 2, a hanging queen and a knight fork. The gate takes the best
of `BENCH_RUNS` runs (default 3) and fails when any of these is true:
- nodes per second fall below `nps_floor`, about 40% of what the baseline machine measured, so slower CI runners still pass
- total nodes exceed `max_total_nodes`, three times the baseline. That would mean pruning or move ordering got much worse
- any tactic is missed

After an intentional engine change, run `node tools/qa/bench-gate.mjs --update` on a typical developer
machine and commit the new `bench-baseline.json`. Add positions to `TACTICS` in
`crates/gm-engine/src/bin/gm-bench-gate.rs`.

## Allowlist (`allowlist.json`)

Intentional exceptions. Every entry carries a `reason`.
- `untranslated`: i18n keys whose translation may equal English. A trailing `*` matches a prefix.
- `visibleEnglish`: English text allowed on non-English pages, such as "Puzzle Rush" and the app name.
- `rawHtmlT`: raw `${t(...)}` allowed in an HTML template, `{ "file": "web/js/…", "key": "a.b", "reason" }` (`"key": "*"` covers the file).
- `sweep`: suppresses sweep issues. `{ "check", "route", "lang", "viewport", "match", "reason" }`, where `route` is the
  pattern from `app.js`, `match` is a substring of the issue detail, and any omitted field matches everything.
  Suppressed issues are still counted in the report. `"check": "skipped"` (with `route`, without `lang`/`viewport`)
  accepts a route the sweep could not visit because no data exists for its parameter; `match` then matches the skip reason.

## CI

`.github/workflows/ci.yml` runs these jobs:

| Job | Steps |
|---|---|
| `rust` | pinned toolchain from `rust-toolchain.toml`, release build, uploads `grandmentor` + `gm-bench-gate` as the `release-binaries` artifact, then test and `clippy -D warnings` |
| `deny` | `cargo deny check` (advisories, bans, sources, licenses) via `EmbarkStudios/cargo-deny-action` |
| `js` | `check-syntax`, `node --test` unit tests and `check-i18n` |
| `bench` | after `rust`: the engine benchmark gate on the prebuilt `gm-bench-gate` (`BENCH_BIN`) |
| `e2e` | after `rust`: the sweep on the prebuilt server (`GM_BIN`) with Chrome from `browser-actions/setup-chrome` |

`bench` and `e2e` download the binaries instead of compiling the workspace again, and restore their executable bit
(artifacts do not keep file permissions). The `e2e` job uploads the sweep report as the `qa-sweep` artifact and adds a
summary to the job page. Third-party actions are pinned to commit SHAs; Dependabot (`.github/dependabot.yml`) updates
them and the Cargo dependencies weekly.
`cargo fmt --check` is not enforced yet because the codebase is not rustfmt-clean. Enable it after a one-off formatting PR.
