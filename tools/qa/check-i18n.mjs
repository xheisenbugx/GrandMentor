#!/usr/bin/env node
// GrandMentor i18n consistency check. Plain Node 22, no dependencies.
//
//   node tools/qa/check-i18n.mjs            # report errors (exit 1) and a warning summary
//   node tools/qa/check-i18n.mjs --verbose  # also list every warning
//   node tools/qa/check-i18n.mjs --json     # machine-readable report on stdout
//
// Errors (fail the check):
//   - a language's flattened key set differs from English (missing or extra keys)
//   - a message's {placeholders} differ from English (plural forms and list items are merged)
//   - a value has a different kind than English (string vs plural vs list)
//   - invalid plural objects (unknown categories, or no `other`), empty strings, non-string values
//   - a locale file that index.js does not import (its keys would silently never load)
//   - a literal key used in web/js (t('a.b'), t(`a.b`)) that English does not define, or a dynamic
//     key prefix (t(`play.modes.${id}.label`) → "play.modes.") that matches no English key
// Warnings (reported, never fail):
//   - list messages whose length differs from English
//   - non-English values identical to English (possible untranslated text), minus tools/qa/allowlist.json
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const WEB = path.join(ROOT, 'web');
const args = new Set(process.argv.slice(2));
const VERBOSE = args.has('--verbose');
const JSON_OUT = args.has('--json');

const PLURAL_KEYS = new Set(['zero', 'one', 'two', 'few', 'many', 'other']);
const errors = [];
const warnings = [];
const err = (lang, key, msg) => errors.push({ lang, key, msg });
const warn = (lang, key, msg) => warnings.push({ lang, key, msg });

function loadAllowlist() {
  const file = path.join(ROOT, 'tools/qa/allowlist.json');
  if (!existsSync(file)) return { untranslated: [] };
  try { return JSON.parse(readFileSync(file, 'utf8')); } catch (e) { err('-', file, `allowlist is not valid JSON: ${e.message}`); return { untranslated: [] }; }
}

const isPlainObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const looksPlural = (v) => isPlainObject(v) && Object.keys(v).length > 0 && Object.keys(v).every((k) => PLURAL_KEYS.has(k));

/** Same flattening rules as web/js/i18n.js, plus validation. Returns Map<key, {kind, value}>. */
function flatten(lang, obj, prefix, out) {
  for (const [k, v] of Object.entries(obj || {})) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (looksPlural(v)) {
      if (!('other' in v)) err(lang, key, 'plural object has no "other" form');
      for (const [form, s] of Object.entries(v)) {
        if (typeof s !== 'string') err(lang, `${key}.${form}`, `plural form must be a string, got ${typeof s}`);
        else if (!s.trim()) err(lang, `${key}.${form}`, 'empty string');
      }
      out.set(key, { kind: 'plural', value: v });
    } else if (isPlainObject(v)) {
      flatten(lang, v, key, out);
    } else if (Array.isArray(v)) {
      v.forEach((s, i) => {
        if (typeof s !== 'string') err(lang, `${key}[${i}]`, `list item must be a string, got ${typeof s}`);
        else if (!s.trim()) err(lang, `${key}[${i}]`, 'empty string');
      });
      if (v.length === 0) err(lang, key, 'empty list');
      out.set(key, { kind: 'list', value: v });
    } else if (typeof v === 'string') {
      if (!v.trim()) err(lang, key, 'empty string');
      out.set(key, { kind: 'string', value: v });
    } else {
      err(lang, key, `value must be a string, list or plural object, got ${v === null ? 'null' : typeof v}`);
    }
  }
  return out;
}

const strings = (entry) => (entry.kind === 'string' ? [entry.value] : entry.kind === 'list' ? entry.value : Object.values(entry.value)).filter((s) => typeof s === 'string');
const placeholders = (entry) => new Set(strings(entry).flatMap((s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1])));

async function loadLanguages() {
  const mod = await import(pathToFileURL(path.join(WEB, 'js/languages.js')).href);
  return mod.LANGUAGES;
}

async function loadCatalog(lang) {
  const dir = path.join(WEB, 'locales', lang);
  const index = path.join(dir, 'index.js');
  if (!existsSync(index)) { err(lang, '-', `missing ${path.relative(ROOT, index)}`); return null; }
  let mod;
  try { mod = await import(pathToFileURL(index).href); } catch (e) { err(lang, '-', `cannot import ${path.relative(ROOT, index)}: ${e.message}`); return null; }
  // Every namespace file must be wired into index.js.
  const files = readdirSync(dir).filter((f) => f.endsWith('.js') && f !== 'index.js').map((f) => f.slice(0, -3));
  const namespaces = new Set(Object.keys(mod.default || {}));
  for (const ns of files) if (!namespaces.has(ns)) err(lang, ns, `web/locales/${lang}/${ns}.js is not imported by index.js`);
  return flatten(lang, mod.default, '', new Map());
}

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = path.join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.js')) out.push(p);
  }
  return out;
}

/** Keys referenced from web/js: literal ('a.b', `a.b`), and dynamic prefixes (`a.${x}`, 'a.' + x). */
function scanCode() {
  const literal = new Map(); // key -> first location
  const prefixes = new Map();
  const re = /(?<![\w$.])t\(\s*(?:(['"])((?:\\.|(?!\1)[^\\\n])*)\1\s*([+,)])|`([^`]*)`)/g;
  for (const file of walk(path.join(WEB, 'js'))) {
    const src = readFileSync(file, 'utf8');
    const rel = path.relative(ROOT, file);
    for (const m of src.matchAll(re)) {
      const lineStart = src.lastIndexOf('\n', m.index) + 1;
      if (/^\s*(\/\/|\*|\/\*)/.test(src.slice(lineStart, m.index))) continue; // comment
      const line = src.slice(0, m.index).split('\n').length;
      const loc = `${rel}:${line}`;
      if (m[4] !== undefined) {
        const tpl = m[4];
        const i = tpl.indexOf('${');
        if (i < 0) { if (!literal.has(tpl)) literal.set(tpl, loc); }
        else if (tpl.slice(0, i).includes('.')) { const pre = tpl.slice(0, i); if (!prefixes.has(pre)) prefixes.set(pre, loc); }
      } else if (m[3] === '+') {
        if (m[2].includes('.') && !prefixes.has(m[2])) prefixes.set(m[2], loc);
      } else if (m[2] && /^[\w-]+(\.[\w-]+)+$/.test(m[2])) {
        if (!literal.has(m[2])) literal.set(m[2], loc);
      }
    }
  }
  return { literal, prefixes };
}

const LANGUAGES = await loadLanguages();
const allow = loadAllowlist();
const allowUntranslated = (allow.untranslated || []).map((e) => (typeof e === 'string' ? e : e.key));
const isAllowedUntranslated = (key) => allowUntranslated.some((p) => (p.endsWith('*') ? key.startsWith(p.slice(0, -1)) : key === p));

const en = await loadCatalog('en');
if (!en) { console.error('Cannot load English catalog'); process.exit(1); }
const langs = Object.keys(LANGUAGES);
for (const dir of readdirSync(path.join(WEB, 'locales'))) {
  if (statSync(path.join(WEB, 'locales', dir)).isDirectory() && !langs.includes(dir)) warn(dir, '-', `web/locales/${dir}/ exists but is not listed in web/js/languages.js`);
}

const stats = { en: en.size };
for (const lang of langs.filter((l) => l !== 'en')) {
  const cat = await loadCatalog(lang);
  if (!cat) continue;
  stats[lang] = cat.size;
  const categories = new Set(new Intl.PluralRules(LANGUAGES[lang].locale).resolvedOptions().pluralCategories);
  for (const [key, e] of en) {
    const o = cat.get(key);
    if (!o) { err(lang, key, 'missing (present in en)'); continue; }
    if (o.kind !== e.kind) { err(lang, key, `is a ${o.kind} but en is a ${e.kind}`); continue; }
    const pe = placeholders(e), po = placeholders(o);
    const missing = [...pe].filter((p) => !po.has(p) && !(e.kind === 'plural' && p === 'count'));
    const extra = [...po].filter((p) => !pe.has(p));
    if (missing.length) err(lang, key, `missing placeholder(s) ${missing.map((p) => `{${p}}`).join(', ')}`);
    if (extra.length) err(lang, key, `unknown placeholder(s) ${extra.map((p) => `{${p}}`).join(', ')} (not in en)`);
    if (e.kind === 'plural') {
      for (const form of Object.keys(o.value)) if (form !== 'zero' && !categories.has(form)) warn(lang, key, `plural form "${form}" is never selected for ${LANGUAGES[lang].locale}`);
      if (categories.has('one') && 'one' in e.value && !('one' in o.value)) err(lang, key, 'plural object lacks the "one" form');
    }
    if (e.kind === 'list' && e.value.length !== o.value.length) warn(lang, key, `list has ${o.value.length} items, en has ${e.value.length}`);
    const same = strings(e).some((s, i) => s.length >= 12 && /[a-z]{3,}\s+[a-z]{3,}/i.test(s) && s === strings(o)[i]);
    if (same && !isAllowedUntranslated(key)) warn(lang, key, 'identical to English (untranslated?)');
  }
  for (const key of cat.keys()) if (!en.has(key)) err(lang, key, 'extra key (not in en)');
}

const { literal, prefixes } = scanCode();
for (const [key, loc] of literal) if (!en.has(key)) err('code', key, `used at ${loc} but not defined in en`);
const enKeys = [...en.keys()];
for (const [pre, loc] of prefixes) if (!enKeys.some((k) => k.startsWith(pre))) err('code', `${pre}*`, `dynamic key prefix used at ${loc} matches no en key`);

const report = { ok: errors.length === 0, languages: langs, keys: stats, codeKeys: literal.size, codePrefixes: prefixes.size, errors, warnings };
if (JSON_OUT) {
  console.log(JSON.stringify(report, null, 2));
} else {
  console.log(`i18n: ${langs.join(', ')} — ${Object.entries(stats).map(([l, n]) => `${l}=${n} keys`).join(', ')}; code uses ${literal.size} literal keys, ${prefixes.size} dynamic prefixes`);
  const group = (list) => {
    const by = new Map();
    for (const x of list) { if (!by.has(x.lang)) by.set(x.lang, []); by.get(x.lang).push(x); }
    for (const [lang, xs] of by) { console.log(`  [${lang}] ${xs.length}`); for (const x of xs) console.log(`    ${x.key}: ${x.msg}`); }
  };
  if (errors.length) { console.log(`\nERRORS (${errors.length}):`); group(errors); }
  if (warnings.length) {
    if (VERBOSE) { console.log(`\nwarnings (${warnings.length}):`); group(warnings); }
    else console.log(`\n${warnings.length} warning(s) (run with --verbose to list them)`);
  }
  console.log(errors.length ? '\ni18n check FAILED' : '\ni18n check passed');
}
process.exit(errors.length ? 1 : 0);
