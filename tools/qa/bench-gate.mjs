#!/usr/bin/env node
// Engine benchmark gate. Plain Node 22, no dependencies.
//
//   cargo build --release -p gm-engine --bin gm-bench-gate
//   node tools/qa/bench-gate.mjs            # compare with tools/qa/bench-baseline.json (exit 1 on regression)
//   node tools/qa/bench-gate.mjs --update   # re-measure and rewrite the baseline (floor = 40% of this machine)
//
// The bench binary searches fixed positions to a fixed depth with fresh tables, so node counts are
// deterministic and only time varies by machine. The gate fails when:
//   - nodes/second (best of BENCH_RUNS runs, default 3) drops below `nps_floor`;
//   - total nodes exceed `max_total_nodes` (search got much less efficient: pruning/ordering regression);
//   - any tactical position's best move is not one of the expected moves.
// Environment: BENCH_BIN (default target/release/gm-bench-gate, built with cargo when missing), BENCH_RUNS.
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const BASELINE = path.join(ROOT, 'tools/qa/bench-baseline.json');
const UPDATE = process.argv.includes('--update');
const RUNS = Math.max(1, Number(process.env.BENCH_RUNS) || 3);
const FLOOR_RATIO = 0.4;
const NODES_SLACK = 3;

function binary() {
  const bin = path.resolve(ROOT, process.env.BENCH_BIN || 'target/release/gm-bench-gate');
  if (!existsSync(bin)) {
    console.log('bench: building gm-bench-gate…');
    execFileSync('cargo', ['build', '--release', '-q', '-p', 'gm-engine', '--bin', 'gm-bench-gate'], { cwd: ROOT, stdio: 'inherit' });
  }
  return bin;
}

const bin = binary();
const runs = [];
for (let i = 0; i < RUNS; i++) runs.push(JSON.parse(execFileSync(bin, [], { cwd: ROOT, encoding: 'utf8' })));
const best = runs.reduce((a, b) => (b.speed.nps > a.speed.nps ? b : a));
const nps = best.speed.nps;
const nodes = best.speed.total_nodes;
console.log(`bench: ${RUNS} run(s), nps ${runs.map((r) => r.speed.nps).join(' / ')} → best ${nps}; total nodes ${nodes}; ${best.speed.total_ms} ms`);
for (const t of best.tactics) console.log(`  ${t.solved ? 'ok  ' : 'FAIL'} ${t.name.padEnd(24)} best ${t.best || '-'} (expected ${(t.expected || []).join('|')}) ${t.score ?? t.error ?? ''}`);

if (UPDATE) {
  const baseline = {
    _comment: 'Engine gate floors for tools/qa/bench-gate.mjs. nps_floor is ~40% of measured.nps so slower CI runners pass while a large regression fails; max_total_nodes allows 3x the measured node count. Regenerate with: node tools/qa/bench-gate.mjs --update',
    nps_floor: Math.round((nps * FLOOR_RATIO) / 1000) * 1000,
    max_total_nodes: nodes * NODES_SLACK,
    measured: { nps, total_nodes: nodes, total_ms: best.speed.total_ms, platform: `${process.platform}-${process.arch}`, date: new Date().toISOString().slice(0, 10) },
  };
  writeFileSync(BASELINE, JSON.stringify(baseline, null, 2) + '\n');
  console.log(`bench: wrote ${path.relative(ROOT, BASELINE)} (nps_floor ${baseline.nps_floor}, max_total_nodes ${baseline.max_total_nodes})`);
  process.exit(0);
}

const base = JSON.parse(readFileSync(BASELINE, 'utf8'));
const failures = [];
if (nps < base.nps_floor) failures.push(`nodes/s ${nps} is below the floor ${base.nps_floor} (baseline machine measured ${base.measured?.nps})`);
if (nodes > base.max_total_nodes) failures.push(`total nodes ${nodes} exceed ${base.max_total_nodes} (baseline ${base.measured?.total_nodes}): the search is much less efficient`);
for (const t of best.tactics) if (!t.solved) failures.push(`tactic "${t.name}": played ${t.best || t.error || '-'}, expected ${(t.expected || []).join(' or ')}`);
if (failures.length) {
  console.log('\nbench gate FAILED:\n' + failures.map((f) => `  - ${f}`).join('\n'));
  process.exit(1);
}
console.log(`\nbench gate passed (nps ${nps} ≥ ${base.nps_floor}, nodes ${nodes} ≤ ${base.max_total_nodes}, ${best.tactics.length}/${best.tactics.length} tactics)`);
