// GrandMentor REST client + EngineClient (live analysis websocket).
// Contract: docs/CONTRACT.md §4–§5.

import { getSetting } from './settings.js';
import { t } from './i18n.js';

const DEFAULT_TIMEOUT_MS = 60000;

/** Error thrown by api.* on network failures or non-2xx responses. */
export class ApiError extends Error {
  constructor(message, status = 0, data = null) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.data = data;
  }
}

/**
 * Translated client-side error text, resolved at call time. Falls back to English when i18n has not
 * loaded yet (an error during boot) so a raw key such as "common.errors.timeout" is never shown.
 */
function tr(key, fallback, params) {
  let v = key;
  try { v = t(key, params); } catch { /* i18n not ready */ }
  return v === key ? fallback : v;
}

/**
 * Tell the PWA layer (pwa.js) whether the server answered. A network failure is only a hint:
 * pwa.js confirms it with a health probe before showing the "engine isn't running" banner.
 */
function signalReach(ok) {
  try { window.dispatchEvent(new Event(ok ? 'gm:server-reachable' : 'gm:server-unreachable')); } catch { /* no window */ }
}

async function request(method, path, body, opts = {}) {
  const ctrl = new AbortController();
  const timeoutMs = opts.timeout ?? DEFAULT_TIMEOUT_MS;
  const timer = timeoutMs > 0 ? setTimeout(() => ctrl.abort(new DOMException('Request timed out', 'TimeoutError')), timeoutMs) : null;
  // Chain a caller-provided AbortSignal (e.g. page cleanup).
  const outer = opts.signal;
  const onOuterAbort = () => ctrl.abort(outer.reason);
  if (outer) {
    if (outer.aborted) ctrl.abort(outer.reason);
    else outer.addEventListener('abort', onOuterAbort, { once: true });
  }

  const init = { method, headers: { Accept: 'application/json', 'Accept-Language': getSetting('language') || 'en' }, signal: ctrl.signal };
  // keepalive: the request outlives the page (progress saves that must land even when the player
  // navigates away right after finishing). Browsers cap keepalive bodies at 64 KB.
  if (opts.keepalive) init.keepalive = true;
  if (body !== undefined) {
    init.headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(body);
  }

  let res;
  try {
    res = await fetch(path, init);
  } catch (e) {
    if (ctrl.signal.aborted) {
      const reason = ctrl.signal.reason;
      if (reason?.name === 'TimeoutError' && !outer?.aborted) throw new ApiError(tr('common.errors.timeout', 'The server took too long to respond.'), 0);
      // Caller aborted: always surface a standard AbortError so callers can ignore it via isAbort().
      throw new DOMException('Request aborted', 'AbortError');
    }
    signalReach(false);
    throw new ApiError(tr('common.errors.unreachable', 'Cannot reach the GrandMentor server. Is it running?'), 0);
  } finally {
    if (timer) clearTimeout(timer);
    if (outer) outer.removeEventListener('abort', onOuterAbort);
  }

  signalReach(true);
  const type = res.headers.get('content-type') || '';
  let data = null;
  try {
    if (res.status === 204) data = null;
    else if (type.includes('application/json')) data = await res.json();
    else data = await res.text();
  } catch {
    data = null;
  }

  if (!res.ok) {
    // Another device whose sign-in was revoked (Settings → Use on your phone): back to the PIN page.
    if (res.status === 401 && data && typeof data === 'object' && data.login === '/login') location.assign('/login');
    const msg = (data && typeof data === 'object' && data.error) ? String(data.error)
      : (typeof data === 'string' && data && data.length < 300) ? data
      : tr('common.errors.requestFailed', `Request failed (${res.status})`, { status: res.status });
    throw new ApiError(msg, res.status, data);
  }
  return data;
}

/**
 * REST client. Paths are absolute like '/api/games'. Throws ApiError(message) on non-2xx.
 * Optional last arg: { signal?: AbortSignal, timeout?: ms, keepalive?: boolean }.
 * Progress writes (attempts, scores, lesson completion) pass { keepalive: true } and no page signal
 * so leaving the page right after finishing doesn't cancel the save; guard only the UI update.
 */
export const api = {
  get: (path, opts) => request('GET', path, undefined, opts),
  post: (path, body = {}, opts) => request('POST', path, body, opts),
  put: (path, body = {}, opts) => request('PUT', path, body, opts),
  del: (path, opts) => request('DELETE', path, undefined, opts),
};

/** Build a query string from an object, skipping null/undefined/'' values. */
export function qs(params = {}) {
  const u = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === null || v === undefined || v === '') continue;
    u.set(k, String(v));
  }
  const s = u.toString();
  return s ? `?${s}` : '';
}

/** Whether the abort came from the caller (safe to ignore). */
export function isAbort(e) {
  return e?.name === 'AbortError';
}

// ---------------------------------------------------------------------------
// EngineClient
// ---------------------------------------------------------------------------

const BACKOFF_BASE_MS = 500;
const BACKOFF_MAX_MS = 10000;
/** Consecutive connection attempts that never opened before giving up and reporting onError. */
const MAX_FAILED_CONNECTS = 5;

/**
 * One lazily-connected websocket to /api/engine/ws.
 *   const engine = new EngineClient();
 *   engine.analyze(fen, { multipv: 3, movetime_ms: 4000 }, (info, done) => { ... });
 *   engine.stop();   engine.close();   // close() in page cleanup!
 * Only the latest analyze() request receives callbacks; stale ids are ignored.
 * Reconnects with exponential backoff while a request is active and re-sends it. After
 * MAX_FAILED_CONNECTS attempts in a row that never open (server down, revoked phone sign-in), it
 * stops, calls the active request's onError(message) and forgets it; the next analyze() tries again.
 */
export class EngineClient {
  constructor({ url } = {}) {
    this._url = url || EngineClient.defaultUrl();
    this._ws = null;
    this._nextId = 1;
    this._active = null;          // { id, msg, onInfo, onError }
    this._closed = false;
    this._attempts = 0;
    this._failedConnects = 0;     // consecutive sockets that closed before 'open'
    this._wsOpened = false;       // whether the current socket reached 'open'
    this._reconnectTimer = null;
    this._onOpen = this._handleOpen.bind(this);
    this._onMessage = this._handleMessage.bind(this);
    this._onClose = this._handleClose.bind(this);
    this._onError = () => { /* close event follows; handled there */ };
  }

  static defaultUrl() {
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    return `${proto}//${location.host}/api/engine/ws`;
  }

  /** True when the socket is open. */
  get connected() {
    return !!this._ws && this._ws.readyState === WebSocket.OPEN;
  }

  /**
   * Start analysing `fen`. Cancels any previous request on this client.
   * onInfo(info, done) — info: {id, depth, nodes, nps, time_ms?, lines:[{score, moves, san}]}.
   * onError(message) optional.
   * @returns {number} request id
   */
  analyze(fen, { multipv = 1, movetime_ms = 3000, depth = null } = {}, onInfo, onError) {
    if (this._closed) throw new Error('EngineClient is closed');
    const id = this._nextId++;
    const msg = { type: 'analyze', id, fen, multipv, movetime_ms, depth };
    this._active = { id, msg, onInfo: typeof onInfo === 'function' ? onInfo : null, onError: typeof onError === 'function' ? onError : null };
    if (this.connected) this._send(msg);
    else this._connect();
    return id;
  }

  /** Stop the current analysis; no further callbacks for it. */
  stop() {
    this._active = null;
    if (this.connected) this._send({ type: 'stop' });
  }

  /** Permanently close: stops analysis, closes the socket, cancels reconnects. */
  close() {
    if (this._closed) return;
    this._closed = true;
    this._active = null;
    if (this._reconnectTimer) { clearTimeout(this._reconnectTimer); this._reconnectTimer = null; }
    this._teardownSocket(true);
  }

  // -- internals ---------------------------------------------------------
  _connect() {
    if (this._closed || this._reconnectTimer) return;
    if (this._ws && (this._ws.readyState === WebSocket.CONNECTING || this._ws.readyState === WebSocket.OPEN)) return;
    let ws;
    try {
      ws = new WebSocket(this._url);
    } catch (e) {
      this._connectFailed();
      return;
    }
    this._ws = ws;
    this._wsOpened = false;
    ws.addEventListener('open', this._onOpen);
    ws.addEventListener('message', this._onMessage);
    ws.addEventListener('close', this._onClose);
    ws.addEventListener('error', this._onError);
  }

  _teardownSocket(sendStop) {
    const ws = this._ws;
    if (!ws) return;
    this._ws = null;
    ws.removeEventListener('open', this._onOpen);
    ws.removeEventListener('message', this._onMessage);
    ws.removeEventListener('close', this._onClose);
    ws.removeEventListener('error', this._onError);
    try {
      if (sendStop && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: 'stop' }));
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) ws.close(1000, 'client closed');
    } catch { /* ignore */ }
  }

  _send(obj) {
    try {
      this._ws.send(JSON.stringify(obj));
    } catch (e) {
      console.warn('[engine] send failed', e);
    }
  }

  _handleOpen() {
    this._attempts = 0;
    this._failedConnects = 0;
    this._wsOpened = true;
    if (this._active) this._send(this._active.msg);
  }

  _handleMessage(ev) {
    let msg;
    try { msg = JSON.parse(ev.data); } catch { return; }
    if (!msg || typeof msg !== 'object') return;
    const a = this._active;
    if (!a || msg.id !== a.id) return; // stale or unsolicited
    if (msg.type === 'info' || msg.type === 'done') {
      const done = msg.type === 'done';
      if (done) this._active = null;
      if (a.onInfo) {
        try { a.onInfo(msg, done); } catch (e) { console.error('[engine] onInfo error', e); }
      }
    } else if (msg.type === 'error') {
      this._active = null;
      const text = msg.message || msg.error || tr('common.errors.engine', 'The engine had a problem.');
      if (a.onError) { try { a.onError(text); } catch (e) { console.error(e); } }
      else console.warn('[engine]', text);
    }
  }

  _handleClose(ev) {
    const opened = this._wsOpened;
    this._wsOpened = false;
    this._teardownSocket(false);
    if (this._closed) return;
    // Dropped while waiting for results (1006 = abnormal): maybe the server stopped.
    if (this._active && ev && ev.code === 1006) signalReach(false);
    if (!opened) { this._connectFailed(); return; }
    // Only reconnect if someone is waiting for results; otherwise connect lazily next time.
    if (this._active) this._scheduleReconnect();
  }

  /** A socket that never opened (refused upgrade, 401, server down): retry a few times, then give up. */
  _connectFailed() {
    if (this._closed) return;
    this._failedConnects++;
    if (this._failedConnects < MAX_FAILED_CONNECTS) {
      if (this._active) this._scheduleReconnect();
      return;
    }
    this._failedConnects = 0;
    this._attempts = 0;
    const a = this._active;
    this._active = null;
    // The upgrade's HTTP status isn't visible to scripts: ask the REST API, which redirects a
    // revoked phone sign-in to /login (and lets pwa.js show the "server isn't running" banner).
    request('GET', '/api/health', undefined, { timeout: 5000 }).catch(() => {});
    if (a && a.onError) {
      try { a.onError(tr('common.errors.engineUnreachable', 'Can’t reach the engine right now. Check that GrandMentor is running, then try again.')); } catch (e) { console.error(e); }
    }
  }

  _scheduleReconnect() {
    if (this._closed || this._reconnectTimer) return;
    const delay = Math.min(BACKOFF_MAX_MS, BACKOFF_BASE_MS * 2 ** this._attempts) * (0.8 + Math.random() * 0.4);
    this._attempts = Math.min(this._attempts + 1, 10);
    this._reconnectTimer = setTimeout(() => {
      this._reconnectTimer = null;
      if (!this._closed && this._active) this._connect();
    }, delay);
  }
}
