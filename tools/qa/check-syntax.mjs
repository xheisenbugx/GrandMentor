#!/usr/bin/env node
// Syntax check for every JavaScript file in web/, tools/ and tests/. Plain Node 22, no dependencies.
//
//   node tools/qa/check-syntax.mjs
//
// ES modules are checked with `node --input-type=module --check` (plain `node --check file.js` does not
// reliably parse these files as modules). Classic scripts are parsed with vm.Script without running them:
//   - web/sw.js: the service worker is registered as a classic script, so `import`/`export` would break it;
//   - tools/qa/page-checks.js: evaluated inside the app page by sweep.mjs as one function expression.
// Exit code: 0 clean, 1 syntax errors.
import { spawnSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const DIRS = ['web', 'tools', 'tests'];
const CLASSIC = new Set(['web/sw.js', 'tools/qa/page-checks.js']);

function walk(dir, out) {
  let entries;
  try { entries = readdirSync(path.join(ROOT, dir), { withFileTypes: true }); } catch { return out; }
  for (const e of entries) {
    const rel = `${dir}/${e.name}`;
    if (e.isDirectory()) { if (e.name !== 'node_modules') walk(rel, out); }
    else if (/\.(m?js)$/.test(e.name)) out.push(rel);
  }
  return out;
}

const files = DIRS.flatMap((d) => walk(d, [])).sort();
let failed = 0;
for (const rel of files) {
  const src = readFileSync(path.join(ROOT, rel), 'utf8');
  let error = null;
  if (CLASSIC.has(rel)) {
    try { new vm.Script(src, { filename: rel }); } catch (e) { error = e.stack || String(e); }
  } else {
    const r = spawnSync(process.execPath, ['--input-type=module', '--check'], { input: src, encoding: 'utf8' });
    if (r.status !== 0) error = (r.stderr || r.stdout || `exit ${r.status}`).trim();
  }
  if (error) { failed++; console.log(`FAIL ${rel}\n${error.replace(/^/gm, '    ')}`); }
}
console.log(`syntax: ${files.length} files (${[...CLASSIC].filter((f) => files.includes(f)).length} classic scripts), ${failed} with errors`);
process.exit(failed ? 1 : 0);
