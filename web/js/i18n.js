// GrandMentor — internationalisation (i18n).
//
// Messages live in web/locales/<lang>/<namespace>.js (plain ES modules exporting an object) and
// are aggregated by web/locales/<lang>/index.js. English is always loaded and is the fallback
// for any key missing in the active language. See docs/I18N.md for conventions.
//
//   t('play.hint')                          → "Hint" / "Pista"
//   t('library.gamesCount', { count: 3 })   → plural-aware via Intl.PluralRules
//   t('home.greeting', { name: 'Alex' })    → "{name}" placeholders are replaced
//
// t() is synchronous: initI18n() must resolve before the first render (app.js awaits it).
// Never call t() at module top level — call it at render time so a language switch re-renders.

import { LANGUAGES, DEFAULT_LANGUAGE, isLanguage } from './languages.js';
import { getSetting, setSetting, onSettingsChange } from './settings.js';

const PLURAL_KEYS = new Set(['zero', 'one', 'two', 'few', 'many', 'other']);
/** @type {Map<string, Map<string, string|object>>} lang -> flat key -> message (bounded: one per language) */
const catalogs = new Map();
const listeners = new Set();
const warned = new Set();
let lang = DEFAULT_LANGUAGE;
let pluralRules = null;
let ready = null;

function isPluralEntry(v) {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return false;
  const keys = Object.keys(v);
  return keys.length > 0 && keys.every((k) => PLURAL_KEYS.has(k)) && 'other' in v;
}

/** Flatten nested namespaces into dotted keys ("play.buttons.hint"); plural objects stay whole. */
function flatten(obj, prefix, out) {
  for (const [k, v] of Object.entries(obj || {})) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === 'object' && !Array.isArray(v) && !isPluralEntry(v)) flatten(v, key, out);
    else out.set(key, v);
  }
  return out;
}

async function loadCatalog(code) {
  if (catalogs.has(code)) return catalogs.get(code);
  const mod = await import(`../locales/${code}/index.js`);
  const flat = flatten(mod.default, '', new Map());
  catalogs.set(code, flat);
  return flat;
}

async function activate(code) {
  const next = isLanguage(code) ? code : DEFAULT_LANGUAGE;
  try {
    await loadCatalog(next);
    lang = next;
  } catch (e) {
    console.error(`[i18n] could not load "${next}", falling back to English`, e);
    lang = DEFAULT_LANGUAGE;
  }
  pluralRules = new Intl.PluralRules(LANGUAGES[lang].locale);
  const root = document.documentElement;
  root.lang = lang;
  root.dir = LANGUAGES[lang].dir || 'ltr';
}

/**
 * Load English + the saved language. Resolves when t() is ready. Safe to call more than once.
 * @returns {Promise<void>}
 */
export function initI18n() {
  if (ready) return ready;
  ready = (async () => {
    await loadCatalog(DEFAULT_LANGUAGE);
    await activate(getSetting('language'));
    onSettingsChange((s, key) => {
      if (key !== 'language' || s.language === lang) return;
      activate(s.language).then(() => {
        for (const fn of Array.from(listeners)) {
          try { fn(lang); } catch (e) { console.error('[i18n] listener error', e); }
        }
      });
    });
  })();
  return ready;
}

function lookup(key) {
  const active = catalogs.get(lang);
  if (active && active.has(key)) return active.get(key);
  const en = catalogs.get(DEFAULT_LANGUAGE);
  if (en && en.has(key)) {
    if (lang !== DEFAULT_LANGUAGE && !warned.has(`${lang}:${key}`)) {
      warned.add(`${lang}:${key}`);
      console.debug(`[i18n] missing "${key}" in ${lang}; using English`);
    }
    return en.get(key);
  }
  if (!warned.has(key)) { warned.add(key); console.warn(`[i18n] unknown key "${key}"`); }
  return undefined;
}

function interpolate(str, params) {
  if (!params) return str;
  return str.replace(/\{(\w+)\}/g, (m, name) => (params[name] === undefined || params[name] === null ? m : String(params[name])));
}

/**
 * Translate a key.
 * @param {string} key dotted key, e.g. "play.hint"
 * @param {Record<string, unknown>} [params] values for {placeholders}; `count` selects plural forms
 * @returns {string}
 */
export function t(key, params) {
  let msg = lookup(key);
  if (msg === undefined) return key;
  if (isPluralEntry(msg)) {
    const n = Number(params?.count ?? 0);
    const form = n === 0 && msg.zero !== undefined ? 'zero' : (pluralRules ? pluralRules.select(n) : 'other');
    msg = msg[form] ?? msg.other;
  }
  if (Array.isArray(msg)) return msg.map((m) => interpolate(String(m), params));
  return interpolate(String(msg), params);
}

/** True when the key exists in the active language or English. */
export function hasKey(key) {
  return catalogs.get(lang)?.has(key) || catalogs.get(DEFAULT_LANGUAGE)?.has(key) || false;
}

/** Current language code, e.g. "en" or "es". */
export function getLanguage() { return lang; }

/** BCP 47 locale for Intl formatting, e.g. "es-ES". */
export function getLocale() { return LANGUAGES[lang].locale; }

/** Change the UI language (persisted in settings). Listeners fire once the catalog is loaded. */
export function setLanguage(code) { return setSetting('language', code); }

/** Subscribe to language changes: fn(langCode). Returns an unsubscribe function. */
export function onLanguageChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Locale-aware number formatting. */
export function formatNumber(n, opts) {
  try { return new Intl.NumberFormat(getLocale(), opts).format(n); } catch { return String(n); }
}

/** Locale-aware date formatting (Intl.DateTimeFormat options). */
export function formatDateIntl(date, opts) {
  const d = date instanceof Date ? date : new Date(date);
  if (Number.isNaN(d.getTime())) return '';
  try { return new Intl.DateTimeFormat(getLocale(), opts).format(d); } catch { return d.toDateString(); }
}

/** Locale-aware list: ["a","b","c"] → "a, b and c" / "a, b y c". */
export function formatList(items, type = 'conjunction') {
  try { return new Intl.ListFormat(getLocale(), { style: 'long', type }).format(items.map(String)); } catch { return items.join(', '); }
}

export { LANGUAGES, DEFAULT_LANGUAGE };
