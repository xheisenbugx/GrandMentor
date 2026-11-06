// Minimal browser globals so web/js modules can be imported by `node --test`.
// Only what the modules touch at import time: localStorage, <html> (dataset/style), event targets,
// matchMedia and location. Nothing renders; `document.readyState` stays "loading" so app.js never boots.
const noop = () => {};

function memoryStorage() {
  const m = new Map();
  return {
    getItem: (k) => (m.has(k) ? m.get(k) : null),
    setItem: (k, v) => { m.set(k, String(v)); },
    removeItem: (k) => { m.delete(k); },
    clear: () => m.clear(),
    get length() { return m.size; },
  };
}

const style = { setProperty: noop, removeProperty: noop, fontSize: '' };
const documentElement = { dataset: {}, style, lang: 'en', dir: 'ltr', classList: { add: noop, remove: noop, toggle: noop, contains: () => false } };

if (!globalThis.localStorage) globalThis.localStorage = memoryStorage();
if (!globalThis.sessionStorage) globalThis.sessionStorage = memoryStorage();
if (!globalThis.document) {
  globalThis.document = {
    documentElement,
    readyState: 'loading', // app.js waits for DOMContentLoaded, which never fires here
    addEventListener: noop,
    removeEventListener: noop,
    querySelector: () => null,
    querySelectorAll: () => [],
    getElementById: () => null,
  };
}
if (!globalThis.window) {
  globalThis.window = {
    addEventListener: noop,
    removeEventListener: noop,
    dispatchEvent: () => true,
    matchMedia: () => ({ matches: false, addEventListener: noop, removeEventListener: noop }),
  };
}
if (!globalThis.location) globalThis.location = { hash: '', href: 'http://localhost/', origin: 'http://localhost', pathname: '/', search: '' };
if (!globalThis.history) globalThis.history = { replaceState: noop, pushState: noop };
