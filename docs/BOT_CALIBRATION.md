# Bot calibration

How the bots' ratings (Pawnny 250 … Titan 3000, and the adaptive bot Sparky) are measured
against the current engine, and how to re-measure them after changing the engine or the bots.

Last calibrated: **2026-10-08** (the JSON stores the UTC date, 2026-10-09; engine at `tools/qa/bench-baseline.json`, ~1.84 M nodes/s on an
Apple M1 Ultra). Machine-readable result: [`crates/gm-bots/calibration.json`](../crates/gm-bots/calibration.json).

## TL;DR

```bash
cargo build --release -p gm-bots --bin gm-calibrate
./target/release/gm-calibrate --threads 8 --seed 5 --out crates/gm-bots/calibration.json
cargo test -p gm-bots --release labels_match_the_committed_calibration
```

A full run plays 5 520 games and takes about 13 minutes on 8 threads (see
[Cost](#cost)). Raw games go to `target/calibration/games.json`; `--fit FILE` refits them
without replaying (e.g. to try another bootstrap count).

## Method

### Players

| Kind | Who | Why |
|---|---|---|
| Ladder bots | the 14 bots from Pawnny to Titan | what we calibrate; their labels anchor the scale |
| Coaches | Mia (`coach`, 1000), Leo (`coach-leo`, 1700) | their labels are shown too |
| Adaptive levels | Sparky at 250, 1000, 1800, 2400, 2800 | checks the level → strength mapping |
| Engine anchors | plain engine, best move at 1 000 / 8 000 / 128 000 nodes | fixed references: they only change when the engine changes, so runs can be compared |
| Random mover | uniformly random legal moves | the floor of the scale |

Players are sorted by expected strength and each one plays the next 3 above it
(`--neighbors`), so every pairing is between players close enough for the result to carry
information, and the comparison graph is connected from the random mover to Titan.

### Games

- **Openings:** the first ≤ 8 plies of every distinct line in `data/openings.json` (221 lines).
  Each opening is played twice with colours reversed (a *game pair*). The bots then continue in
  their own opening book, exactly as in the app.
- **Determinism:** each game has its own seed derived from `--seed`. The bots' wall-clock limit
  (`movetime_ms` in `strength.rs`) is converted into a node budget at `--ref-nps` (default
  1 800 000, the bench baseline machine) so a game depends only on its seed, never on machine
  load. The same command reproduces the same games on any machine; only the wall time differs.
  (In the app the wall-clock limit applies, so on a machine much slower than the baseline the
  strongest bots — the only ones whose time limit binds before their node limit — play a little
  weaker.)
- **Fresh engines per game** (32 MB hash per side, like the server's default).
- **Adjudication** (a 6 000-node referee search after every ply):
  - checkmate, stalemate, insufficient material, 50-move rule, threefold repetition: as in chess;
  - *resign*: referee eval beyond ±10 pawns for 8 plies in a row **and** the winning side is
    nominally ≥ 1200 (weaker bots must actually convert: failing to mate is part of their level);
  - *draw*: after ply 120, eval within ±0.2 pawns for 16 plies, both players nominally ≥ 1600;
  - *ply cap* (300): win for the side ahead by ≥ 3 pawns, otherwise a draw.

### Rating fit

- **Model:** Elo logistic / Bradley–Terry, `P(A beats B) = 1 / (1 + 10^((Rb − Ra)/400))`, draws
  count half. Ratings are the **maximum-likelihood** solution over all games (Newton's method,
  `crates/gm-bots/src/calibration.rs`) with a weak prior of half a virtual draw per pairing so a
  100 % score stays finite.
- **Anchor:** Elo only measures differences. We shift the scale so that the **mean rating of the
  14 ladder bots equals the mean of their labels** (1 539). This keeps the numbers comparable to
  the labels users see without privileging any single bot: anchoring the weakest bot at 250 would
  push all of its noise onto everyone else, and an engine anchor has no meaningful human rating.
  Consequence: the check is about the *shape* of the ladder (spacing and order); the absolute
  level is "human-ish" only in the sense that it continues the existing labels.
- **Confidence intervals:** 95 % percentile bootstrap (400 resamples) over **game pairs**
  (the two games of an opening are not independent), refitting and re-anchoring each time.

### Tuning: labels stay, strength moves

The labels form an evenly spaced, strictly increasing ladder, so we keep them and instead tune
the **strength setting** each bot plays with (`PLAY_ELO` in `crates/gm-bots/src/strength.rs`;
the Elo fed to `Strength::for_elo`, which sets depth, nodes, MultiPV, temperature, oversight
rate…). Style matters (e.g. the defensive Benny trades down and plays below his nominal
strength), so bots with the same label can need different settings. The adaptive bot uses a
piecewise-linear level → strength curve (`ADAPTIVE_CURVE`) fitted to the Sparky levels.

Procedure used: run once with every bot at its label; estimate the local slope of measured
rating vs strength setting from neighbouring bots and Sparky levels; move each bot's setting by
`(target − measured) / slope`; verify with a fresh run on a **different seed**.

## Results

Final run: `gm-calibrate --threads 8 --seed 5` (80 games per pairing, 5 520 games, average 98.6
plies). "Before" is the first run with every bot playing at its label (`--seed 1`, 40 games per
pairing, same players minus Sparky@2400). Ratings are anchored as described above; intervals are
95 %.

| Player | Label | Before: measured (95 % CI) | After: measured (95 % CI) | After − label |
|---|---:|---:|---:|---:|
| random mover | – | −521 (−761…−378) | −715 (−817…−640) | |
| Pawnny | 250 | 166 (−5…275) | 259 (158…324) | +9 |
| Lulu | 450 | 318 (163…403) | 449 (361…525) | −1 |
| Benny | 650 | 443 (317…527) | 679 (609…737) | +29 |
| Rosa | 850 | 776 (670…852) | 843 (778…901) | −7 |
| Mia (coach) | 1000 | 913 (818…992) | 1003 (946…1060) | +3 |
| Tito | 1000 | 937 (848…1011) | 969 (909…1019) | −31 |
| Max | 1200 | 1122 (1040…1190) | 1235 (1184…1280) | +35 |
| Zara | 1400 | 1349 (1271…1413) | 1403 (1360…1448) | +3 |
| Oliver | 1600 | 1510 (1443…1585) | 1600 (1554…1656) | +0 |
| Leo (coach) | 1700 | 1687 (1621…1763) | 1692 (1647…1752) | −8 |
| Nina | 1800 | 1785 (1722…1863) | 1816 (1771…1870) | +16 |
| Viktor | 2000 | 1953 (1882…2043) | 1992 (1944…2051) | −8 |
| Sofia | 2200 | 2277 (2190…2375) | 2179 (2129…2242) | −21 |
| Kai | 2450 | 2496 (2411…2634) | 2410 (2347…2480) | −40 |
| Athena | 2700 | **2931** (2838…3073) | 2689 (2623…2762) | −11 |
| Titan | 3000 | **3487** (3373…3678) | 3027 (2952…3108) | +27 |
| Sparky @250 | (250) | 188 (17…285) | 226 (128…299) | −24 |
| Sparky @1000 | (1000) | 922 (821…1001) | 962 (901…1014) | −38 |
| Sparky @1800 | (1800) | 1771 (1703…1860) | 1787 (1739…1843) | −13 |
| Sparky @2400 | (2400) | – | 2379 (2324…2446) | −21 |
| Sparky @2800 | (2800) | **3224** (3132…3376) | 2720 (2647…2802) | −80 |
| engine @1 000 nodes | – | 1771 (1705…1856) | 1798 (1754…1851) | |
| engine @8 000 nodes | – | 2305 (2223…2415) | 2233 (2176…2298) | |
| engine @128 000 nodes | – | 2962 (2867…3108) | 2998 (2932…3084) | |

**Findings.**
- The middle of the ladder (Rosa … Viktor) was already close to its labels (mostly 15–90 points
  low, within noise).
- The **top was badly stretched**: with the current engine, the 2700 and 3000 settings play
  ~230 and ~490 points above their labels (Sparky's 2800 level ~420 above). Above ~2200 the
  measured rating rises 1.5–3× faster than the setting, so the old table's top rows were tuned
  for a much weaker engine.
- The **bottom was too weak**: Benny (defensive style: trades and passivity) ~200 below, Lulu
  ~130 below.
- With the anchor fixed by the label mean, a stretched top also drags every other bot's measured
  number down a little; fixing the top fixed most of the small negative offsets.

**Changes (labels unchanged, strength settings moved):**

| Bot | Label | Strength setting before → after |
|---|---:|---:|
| Pawnny | 250 | 250 → 310 |
| Lulu | 450 | 450 → 570 |
| Benny | 650 | 650 → 800 |
| Rosa | 850 | 850 → 915 |
| Mia (coach) | 1000 | 1000 → 1055 |
| Tito | 1000 | 1000 → 1050 |
| Max | 1200 | 1200 → 1225 |
| Zara | 1400 | 1400 (unchanged) |
| Oliver | 1600 | 1600 → 1595 |
| Leo (coach) | 1700 | 1700 → 1695 |
| Nina | 1800 | 1800 → 1770 |
| Viktor | 2000 | 2000 → 2005 |
| Sofia | 2200 | 2200 → 2150 |
| Kai | 2450 | 2450 → 2395 |
| Athena | 2700 | 2700 → 2540 |
| Titan | 3000 | 3000 → 2680 |

Adaptive bot (`ADAPTIVE_CURVE`, level → setting): 250 → 250, 1000 → 1060, 1800 → 1776,
2400 → 2315, 2800 → 2600 (was the identity). It is strictly increasing, spans the whole ladder
(measured 226 → 2720 for levels 250 → 2800) and is unit-tested for monotonicity.

After tuning every label lies inside its bot's 95 % interval, the measured ratings are strictly
increasing along the ladder, and the largest deviation is 40 points (Kai). The engine anchors
each landed inside its "before" interval (shifts of 27–72 points), so the shared scale held.

**Caveats.** Ratings come from bot-vs-bot games; "human-ish" means "continuous with the existing
labels", not validated against real players. Neighbouring bots are now about the advertised
distance apart, which is what a learner climbing the ladder notices. Titan's and Athena's
strength in the app depends on the machine: below ~1.8 M nodes/s their time limit binds first
and they play somewhat weaker than measured. The strength table itself (`TABLE` in
`strength.rs`) is unchanged.

## Cost

| Run | Games | Threads | Wall time | Machine load |
|---|---:|---:|---:|---|
| before (`--games 40`, all bots at label) | 2 640 | 8 | 39 min | heavy (load average 50–90 from parallel builds) |
| final (`--games 80`, default) | 5 520 | 8 | 13 min | moderate (load average ~15) |

The cost is dominated by Titan/Athena games (≈1–1.5 M nodes per move). Because time limits are
node budgets, the games and results are identical on any machine; only wall time varies.
`--games 40` halves the time at the price of ~1.4× wider intervals.

## Re-running

1. `cargo build --release -p gm-bots --bin gm-calibrate`
2. `./target/release/gm-calibrate --threads 8 --seed <new> --out target/calibration/run.json`
3. Compare `rating` with `label`. If a bot is outside its interval, move its `PLAY_ELO` entry by
   `(label − rating) / slope` (slope ≈ 1 below 2200, ≈ 1.5–1.8 above) and run again.
4. When happy, copy the run to `crates/gm-bots/calibration.json`, update this page and run
   `cargo test -p gm-bots --release` (the `labels_match_the_committed_calibration` test checks that
   labels are strictly increasing, measured ratings are in ladder order, every label sits within
   its measured 95 % interval (+25), and the Sparky levels are monotonic).

Re-calibrate whenever the engine's strength changes noticeably (search, eval, NPS), the strength
table changes, or a bot is added or relabelled. Quick experiments: `--only id1,id2,…` and
`--games 16` give a rough picture of the cheap bots in under a minute.
