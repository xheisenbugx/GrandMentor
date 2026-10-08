// Generate a few genuine games (bot vs bot through the real API) so stats pages have data.
import { Chess } from '../../web/vendor/chess.js';
const BASE = process.env.BASE || 'http://localhost:8097';
const post = async (p, b) => (await fetch(BASE + p, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(b) })).json();
const matchups = [['oliver', 'max', 'white'], ['nina', 'rosa', 'black'], ['zara', 'tito', 'white'], ['max', 'zara', 'black'], ['viktor', 'benny', 'white'], ['oliver', 'nina', 'white'], ['nina', 'oliver', 'black']];
for (const [me, opp, color] of matchups) {
  const c = new Chess(); const moves = [];
  while (!c.isGameOver() && moves.length < 160) {
    const myTurn = (c.turn() === 'w') === (color === 'white');
    const r = await post('/api/bot/move', { bot_id: myTurn ? me : opp, start_fen: 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1', moves });
    if (!r.uci) break;
    c.move({ from: r.uci.slice(0, 2), to: r.uci.slice(2, 4), promotion: r.uci[4] }); moves.push(r.uci);
  }
  const result = c.isCheckmate() ? (c.turn() === 'w' ? '0-1' : '1-0') : '1/2-1/2';
  const term = c.isCheckmate() ? 'checkmate' : c.isStalemate() ? 'stalemate' : c.isThreefoldRepetition() ? 'repetition' : 'draw';
  const look = await (await fetch(BASE + '/api/openings/lookup?fen=' + encodeURIComponent(new Chess().fen()))).json();
  const g = await post('/api/games', { white: color === 'white' ? 'Alex' : opp[0].toUpperCase() + opp.slice(1), black: color === 'black' ? 'Alex' : opp[0].toUpperCase() + opp.slice(1),
    result, termination: term, start_fen: 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1', moves, bot_id: opp, user_color: color, time_control: '10+0', opening_name: null, notes: '', tags: [] });
  await post('/api/review', { game_id: g.id });
  console.log(me, 'vs', opp, color, result, moves.length);
}
