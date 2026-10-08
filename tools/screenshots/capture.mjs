import { launch, sleep, BASE } from './cdp.mjs';
import { mkdirSync } from 'node:fs';

const SCRATCH = process.argv[2];
const OUT = process.argv[3];
const ONLY = process.argv[4] ? process.argv[4].split(',') : null;
mkdirSync(OUT, { recursive: true });
const api = async (p, body, method) => {
  const r = await fetch(BASE + p, body ? { method: method || 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {});
  return r.json();
};

// ---- seed a realistic profile ----
const OPERA = `[Event "Paris Opera"]
[Site "Paris"]
[Date "1858.??.??"]
[White "Paul Morphy"]
[Black "Duke Karl / Count Isouard"]
[Result "1-0"]

1. e4 e5 2. Nf3 d6 3. d4 Bg4 4. dxe5 Bxf3 5. Qxf3 dxe5 6. Bc4 Nf6 7. Qb3 Qe7
8. Nc3 c6 9. Bg5 b5 10. Nxb5 cxb5 11. Bxb5+ Nbd7 12. O-O-O Rd8 13. Rxd7 Rxd7
14. Rd1 Qe6 15. Bxd7+ Nxd7 16. Qb8+ Nxb8 17. Rd8# 1-0`;
let games = await api('/api/games');
let opera = games.find((g) => g.white === 'Paul Morphy');
if (!opera) {
  await api('/api/profile', { name: 'Alex' }, 'PUT');
  const [g] = await api('/api/games/import', { pgn: OPERA });
  opera = g;
  await api('/api/review', { game_id: g.id });
  const ps = await api('/api/puzzles/rush?count=12');
  for (const [i, p] of ps.entries()) await api(`/api/puzzles/${p.id}/attempt`, { solved: i % 4 !== 3, time_ms: 20000 });
  await api('/api/progress', { course_id: 'chess-basics', lesson_id: 'the-board', completed: true });
  await api('/api/progress', { course_id: 'chess-basics', lesson_id: 'rook-bishop-queen', completed: true });
}
const want = (n) => !ONLY || ONLY.includes(n);

const b = await launch(SCRATCH);
await b.size(1440, 900);
await b.open(BASE + '/#/', 2500);

if (want('home')) { await b.go('#/', 2500); await b.shot(`${OUT}/home.png`); }

if (want('play')) {
  await b.eval(`localStorage.clear()`);
  await b.go('#/play', 2000);
  await b.clickText('button', 'White');
  await b.clickText('button', 'Friendly');
  await sleep(300);
  await b.shot(`${OUT}/play-bots.png`);
  await b.record(`${OUT}/play.gif`, async () => {
    await sleep(800);
    await b.clickText('button', 'Play Pawnny');
    await sleep(1800);
    for (const mv of ['e2e4', 'g1f3', 'f1c4']) {
      await b.move(mv, 450);
      await sleep(4200); // coach feedback + bot thinking
    }
    await b.clickText('button', 'Hint'); await sleep(1300);
    await b.clickText('button', 'Hint'); await sleep(2500);
  }, { fps: 6, width: 1000, tail: 1200 });
  await b.shot(`${OUT}/play.png`);
}

if (want('review')) {
  await b.go(`#/review/${opera.id}`, 1000);
  await b.waitFor('.rv-start', 30000); await sleep(1200);
  await b.shot(`${OUT}/review.png`);
  await b.click('.rv-start'); await sleep(800);
  for (let i = 0; i < 24; i++) { await b.key('ArrowRight'); await sleep(60); }
  await sleep(800);
  await b.record(`${OUT}/review.gif`, async () => {
    for (let i = 0; i < 9; i++) { await b.key('ArrowRight'); await sleep(2100); }
  }, { fps: 5, width: 1000, tail: 1500 });
  await b.shot(`${OUT}/review-walk.png`);
}

if (want('analysis')) {
  const fen = '3rkb1r/p2nqppp/5n2/1B2p1B1/4P3/1Q6/PPP2PPP/2KR3R w k - 3 13';
  await b.go(`#/analysis?fen=${encodeURIComponent(fen)}`, 6000);
  await b.shot(`${OUT}/analysis.png`);
}

if (want('puzzle')) {
  const daily = await api('/api/puzzles/daily');
  await b.go('#/puzzles/daily', 600);
  await b.record(`${OUT}/puzzle.gif`, async () => {
    await sleep(2800); // opponent's setup move animates in
    for (let i = 1; i < daily.moves.length; i += 2) { await b.move(daily.moves[i], 600); await sleep(2600); }
  }, { fps: 6, width: 1000, tail: 1800 });
  await b.go('#/puzzles', 1800); await b.shot(`${OUT}/puzzles.png`);
}

if (want('learn')) {
  await b.go('#/learn', 1800); await b.shot(`${OUT}/learn.png`);
  await b.go('#/learn/tactics-fundamentals/forks', 2200); await b.shot(`${OUT}/lesson.png`);
}
if (want('openings')) { await b.go('#/openings/italian-game', 2500); await b.shot(`${OUT}/openings.png`); }
if (want('endgames')) { await b.go('#/endgames', 1800); await b.shot(`${OUT}/endgames.png`); }
if (want('profile')) { await b.go('#/profile', 2200); await b.shot(`${OUT}/profile.png`); }
if (want('library')) { await b.go('#/library', 6000); await b.shot(`${OUT}/library.png`); }
if (want('light')) {
  await b.eval(`document.documentElement.dataset.theme = 'light'`);
  await b.go('#/settings', 1800); await b.shot(`${OUT}/settings-light.png`);
  await b.eval(`document.documentElement.dataset.theme = 'dark'`);
}
if (want('mobile')) {
  await b.size(390, 844, true);
  await b.go('#/review/' + opera.id, 4000); await b.shot(`${OUT}/mobile-review.png`);
  await b.go('#/', 2500); await b.shot(`${OUT}/mobile-home.png`);
}
b.close();
console.log('done');
