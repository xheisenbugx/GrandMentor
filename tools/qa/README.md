# Quality gate (`tools/qa/`)

Automated versions of the manual checks in the Definition of done. Plain Node 22 scripts, no npm
dependencies. The same commands run in GitHub Actions (`.github/workflows/ci.yml`) on every pull
request to `dev`/`main` and every push to `dev`.

| Script | What it checks | Needs |
|---|---|---|
| `check-i18n.mjs` | Locale catalogs and `t()` keys used in code | Node |
| `sweep.mjs` | Every route × language × viewport in headless Chrome | release server build, Chrome |
| `bench-gate.mjs` | Engine speed and tactical strength against `bench-baseline.json` | `gm-bench-gate` release build |

## Run it before opening a PR

```bash
cargo build --release --workspace
node tools/qa/check-i18n.mjs          # a few seconds
node tools/qa/bench-gate.mjs          # about 5 s
node tools/qa/sweep.mjs               # about 2 min; starts its own server on a throwaway database
```

Each script exits with `0` when clean and `1` when it found problems. `sweep.mjs` exits with `2` when it
could not run at all (no Chrome, server did not start).

## `check-i18n.mjs`

Loads `web/locales/<lang>/index.js` for every language in `web/js/languages.js` and compares it with English.

**Errors:**
- missing or extra keys
- `{placeholders}` that differ from English
- string, list and plural mismatches
- plural objects without `other`
- empty strings
- locale files that `index.js` does not import
- keys used in `web/js` that English does not define. This covers `t('a.b')` and `` t(`a.b`) ``. Dynamic keys such as `` t(`play.modes.${id}.label`) `` or `t('nav.' + k)` are checked by prefix.

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
3. Skips a route and lists it in the report when no data exists for it.
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
| `english` | non-English pages showing an English catalog phrase (≥ 14 characters, not shared with the target catalog) |
| `hscroll` | the page scrolls horizontally |
| `overflow` | a visible element sticks out of the viewport. Page-level `overflow-x: hidden/clip` does not hide it, but real scroll containers do |
| `layer-over-board` | a speech bubble, toast, popover or tooltip covers a `.gm-board` |
| `button-overlap` | two buttons overlap. The check skips nested buttons and buttons in different fixed or sticky layers |

Output goes to `QA_OUT` (default `target/qa-sweep/`):
- `report.json`
- `report.md`
- `server.log`
- `shots/<lang>-<viewport>-<route>.png` for pages with issues

| Variable | Default | |
|---|---|---|
| `BASE` | – | test an already running server instead of starting one. It seeds a game when the database has none, so point it at a throwaway database |
| `QA_PORT` | `8199` | port of the server the sweep starts (`GM_BIN` overrides the binary) |
| `CDP_PORT` | `9555` | Chrome DevTools port |
| `CHROME`, `CHROME_ARGS` | auto | Chrome executable and extra flags. Detection order: macOS app, then `google-chrome`, then `chromium`. On Linux `--no-sandbox --disable-dev-shm-usage` is the default |
| `QA_LANGS` | all | e.g. `en` or `en,es` |
| `QA_VIEWPORTS` | `desktop,mobile` | |
| `QA_ROUTES` | all | regex over route patterns, e.g. `'^/(play\|review)'` |
| `QA_SAMPLE` | `1` | `N` > 1: non-English languages visit only every N-th route (rotating), to save CI time |
| `QA_SHOTS` | `failures` | `all` or `none` |
| `QA_SETTLE_MS` | `500` | extra wait after the network goes idle |
| `QA_INJECT` | – | JS run in every page, to test the sweep itself: `QA_INJECT="console.error('x')"` must fail every page |

## `bench-gate.mjs`

`cargo run --release -p gm-engine --bin gm-bench-gate` searches reference positions to a fixed depth.
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
- `sweep`: suppresses sweep issues. `{ "check", "route", "lang", "viewport", "match", "reason" }`, where `route` is the
  pattern from `app.js`, `match` is a substring of the issue detail, and any omitted field matches everything.
  Suppressed issues are still counted in the report.

## CI

`.github/workflows/ci.yml` runs these jobs:

| Job | Steps |
|---|---|
| `rust` | build, test, and `clippy -D warnings` |
| `js` | module syntax check and `check-i18n` |
| `bench` | the engine benchmark gate |
| `e2e` | release build, then the sweep with Chrome from `browser-actions/setup-chrome` |

The `e2e` job uploads the sweep report as the `qa-sweep` artifact and adds a summary to the job page.
`cargo fmt --check` is not enforced yet because the codebase is not rustfmt-clean. Enable it after a one-off formatting PR.
