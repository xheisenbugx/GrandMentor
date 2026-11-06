<!-- PRs must target the `dev` branch. See AGENTS.md. -->

## In plain words

<!-- For a non-technical reader: what does this change for someone using GrandMentor, and why does it matter?
     No code, file names or jargon. 2-4 sentences. -->

## Evidence

<!-- At least one screenshot or GIF showing the change (UI changes always).
     Backend-only: a screenshot of the affected screen, or tests / benchmark / API output.
     Tip: tools/screenshots/ captures the real app in headless Chrome. -->

## What changed

<!-- Technical summary for reviewers. -->

## How it was tested

- [ ] `cargo test --workspace --release`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo deny check` (dependency changes)
- [ ] `node tools/qa/check-syntax.mjs` (JS syntax)
- [ ] `node --test 'tests/js/**/*.test.mjs'` (JS unit tests)
- [ ] `node tools/qa/check-i18n.mjs`
- [ ] `node tools/qa/sweep.mjs`
- [ ] `node tools/qa/bench-gate.mjs` (engine changes)
- [ ] Checked in a browser (desktop + mobile)
