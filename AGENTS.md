# AGENTS.md

Guidance for AI coding agents (and humans) working on GrandMentor. Read this before changing code.

## What this project is

GrandMentor is a local-first chess learning web app inspired by chess.com: play bots, live eval bar,
game review with move classifications, a mentor, puzzles, lessons, openings and endgame drills.

- **Backend:** Rust cargo workspace in `crates/` (axum, tokio, shakmaty, rusqlite bundled). The binary is `grandmentor` (crate `gm-server`).
- **Frontend:** plain ES modules in `web/`, **no build step, no npm dependencies**. `web/vendor/chess.js` is vendored.
- **Content:** `data/*.json` (openings, puzzles, courses, endgames), validated at load time.
- **Contract:** [`docs/CONTRACT.md`](docs/CONTRACT.md) is the source of truth for every REST/WebSocket endpoint, JSON shape and frontend component API. Update it in the same PR whenever you change an interface.
- More docs: [`docs/FEATURES.md`](docs/FEATURES.md) (chess.com feature research), [`docs/STYLEGUIDE.md`](docs/STYLEGUIDE.md) (design tokens and CSS classes).

## Commands

```bash
cargo run --release -p gm-server                 # http://localhost:8080 (run from repo root)
cargo build --release --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets           # must stay warning-free
cargo test -p gm-content                         # validates every move in data/*.json and every translation overlay
cargo deny check                                 # advisories, licenses, sources (install: cargo install cargo-deny --locked)
node tools/qa/check-syntax.mjs                   # syntax of every JS file in web/, tools/, tests/
node --test 'tests/js/**/*.test.mjs'             # JS unit tests (zero dependencies)
```

Note: plain `node --check file.js` does **not** reliably catch syntax errors in these module files. `check-syntax.mjs`
uses `node --input-type=module --check` for modules and parses `web/sw.js` (a classic service-worker script) as a script.

The toolchain is pinned in `rust-toolchain.toml` (rustup picks it up automatically; CI uses the same file).
The minimum supported Rust is `rust-version` in `Cargo.toml`.

## Git & pull request workflow

These rules are mandatory.

1. **Branches.** `dev` is the default and integration branch; `main` holds releases. Create a branch from `dev`
   (`feat/...`, `fix/...`, `docs/...`, `chore/...`) and **always open PRs with `dev` as the base branch**
   (`gh pr create --base dev`). Never push directly to `dev` or `main`, and never open a PR against `main`
   unless it is an explicit release PR from `dev`.
2. **Commit identity.** Every commit in this repo must be authored and committed as
   `Osvaldo Cordova Aburto <oca159@hotmail.es>`. The repo-local git config sets this; do not override it, and never
   commit with any other email address (in particular no work addresses). Check with `git log --format='%an <%ae> | %cn <%ce>'`
   before pushing.
3. **PR description.** Every PR follows [`.github/pull_request_template.md`](.github/pull_request_template.md) and **must** include:
   - **"In plain words"** — a short explanation of the PR's purpose for a non-technical reader: what changes for
     the person using the app and why it matters. No code, file names or jargon.
   - **Evidence** — at least one screenshot or GIF showing the change whenever possible (UI changes always).
     For backend-only changes, a screenshot of the affected screen, or a terminal capture of tests, benchmarks or
     API output. **Don't commit PR-only evidence to the code branch**: publish it to the orphan `media` branch
     with `tools/screenshots/publish-evidence.sh <branch-name> <files...>`, which prints the URLs to embed, e.g.
     `https://raw.githubusercontent.com/xheisenbugx/GrandMentor/media/prs/<branch-name>/after.png`
     (slashes in the branch name become `-`). Only images the README or docs actually use go in `docs/media/`.
   - What changed (technical summary) and how it was tested.
4. **Capturing evidence.** Use [`tools/screenshots/`](tools/screenshots/README.md): it drives the real app in
   headless Chrome and produces PNGs and GIFs. Run it against a throwaway database (`GM_DB=/tmp/...`), never your real one.

## Code conventions

**Rust**
- Rust 2021, `unsafe` is forbidden workspace-wide. No `unwrap()`/`expect()` on user-controlled input; map errors to `{ "error": "..." }` JSON with a proper status code.
- Everything is bounded: fixed-size transposition tables, capped LRU caches, request size limits, capped search times.
- Never block the async runtime: engine searches go through `EnginePool::with_engine`, SQLite through blocking tasks.
- Every search must be stoppable through its `AtomicBool` stop flag.
- Moves on the wire are UCI (`e2e4`, `e7e8q`); scores are white-POV `{"cp": n}` / `{"mate": n}`; FENs are full 6-field FENs.

**Frontend**
- Each page in `web/js/pages/` exports `mount(root, { params, query })` and returns a cleanup function that removes **all** listeners, timers, observers, fetches (AbortController) and sockets. Every component has `destroy()`. Memory leaks are bugs.
- Use the design tokens and classes from `docs/STYLEGUIDE.md` and the helpers in `web/js/ui.js`; don't hard-code colours.
- Keep it dependency-free; don't add a bundler or framework.
- Beginner-friendly copy: short, warm and jargon-free.
- **Every user-visible string goes through `t()`** (`web/js/i18n.js`) with keys in `web/locales/en/` **and** `web/locales/es/`. Never hard-code UI text, and never call `t()` at module top level. Server-generated text takes the request's `Lang`. See [`docs/I18N.md`](docs/I18N.md).

**Content (`data/*.json`)**
- Every move must be legal; `cargo test -p gm-content` must pass and the loader must not drop any entry.
- Puzzles come from the CC0 Lichess database; keep `data/ATTRIBUTION.md` accurate.

## Quality gate

CI (`.github/workflows/ci.yml`) runs these on every PR to `dev`/`main`. Run them locally before opening a PR
(details in [`tools/qa/README.md`](tools/qa/README.md)):

```bash
cargo build --release --workspace --locked
cargo test --workspace --release --locked
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check                 # RustSec advisories, licenses, crate sources (deny.toml)
node tools/qa/check-syntax.mjs   # every JS file parses
node --test 'tests/js/**/*.test.mjs'   # JS unit tests for pure modules (tests/js/)
node tools/qa/check-i18n.mjs     # every language has the same keys/placeholders as English; t() keys exist
node tools/qa/bench-gate.mjs     # builds gm-bench-gate, then engine speed floor + tactical positions (tools/qa/bench-baseline.json)
node tools/qa/sweep.mjs          # builds the server, then every route × language × desktop/mobile in headless Chrome, own DB and port
```

The sweep fails on routes it had to skip for lack of data, console errors, failed API calls, horizontal scroll, elements sticking out of the viewport,
raw i18n keys, stray "null"/"undefined" text, oversized icons, English text on non-English pages, bubbles/toasts covering the board and overlapping buttons.
Its report (`target/qa-sweep/report.md` + screenshots) shows what to fix. Intentional exceptions go in
`tools/qa/allowlist.json`, each with a reason. New routes in `web/js/app.js` are swept automatically.

## Definition of done

- [ ] `cargo build`, `cargo test --workspace --release`, `cargo clippy` and `cargo deny check` are clean
- [ ] `node tools/qa/check-syntax.mjs` and `node --test 'tests/js/**/*.test.mjs'` pass
- [ ] `node tools/qa/check-i18n.mjs` and `node tools/qa/sweep.mjs` pass (and `node tools/qa/bench-gate.mjs` for engine changes)
- [ ] UI changes checked in a real browser (desktop and ~390px mobile), no console errors
- [ ] `docs/CONTRACT.md` updated if an interface changed
- [ ] New UI text translated in every language under `web/locales/` (English is the fallback)
- [ ] PR targets `dev`, has an **In plain words** section and screenshot/GIF evidence
