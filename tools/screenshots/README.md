# Screenshot & GIF capture

Drives the real app in headless Chrome over the DevTools Protocol and writes PNG screenshots
and GIFs (via `ffmpeg`). Used for the README and as visual evidence in pull requests.

Requirements: Google Chrome, Node 22+, `ffmpeg`, and ImageMagick (`magick`, optional, for compressing PNGs).

```bash
# 1. Run a server against a throwaway database
cargo build --release
GM_PORT=8097 GM_DB=/tmp/gm-shots.db ./target/release/grandmentor &

# 2. Optional: generate a few genuine bot-vs-bot games so stats pages have data
node tools/screenshots/seed-games.mjs

# 3. Capture everything, or a comma-separated subset of scenes
node tools/screenshots/capture.mjs /tmp/gm-chrome /tmp/gm-shots
node tools/screenshots/capture.mjs /tmp/gm-chrome /tmp/gm-shots play,review
```

Scenes: `home play review analysis puzzle learn openings endgames profile library light mobile`.
`BASE` overrides the server URL (default `http://localhost:8097`).
Use `b.shot()` for a screenshot and `b.record(file, async () => { ... })` for a GIF when adding scenes.
