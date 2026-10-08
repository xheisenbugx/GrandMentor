// Minimal Chrome DevTools Protocol driver used to capture README screenshots / GIFs.
import { spawn, execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import path from 'node:path';

export const BASE = process.env.BASE || 'http://localhost:8097';
const CDP_PORT = Number(process.env.CDP_PORT) || 9333;
const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export { sleep };

export async function launch(scratch, width = 1440, height = 900) {
  const profile = path.join(scratch, 'chrome-profile');
  rmSync(profile, { recursive: true, force: true });
  const proc = spawn(CHROME, ['--headless=new', `--remote-debugging-port=${CDP_PORT}`, `--user-data-dir=${profile}`,
    '--no-first-run', '--hide-scrollbars', '--force-device-scale-factor=1', `--window-size=${width},${height}`, 'about:blank'], { stdio: 'ignore' });
  let targets;
  for (let i = 0; i < 50; i++) {
    try { targets = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json`)).json(); if (targets.find((t) => t.type === 'page')) break; } catch {}
    await sleep(200);
  }
  const page = targets.find((t) => t.type === 'page');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r) => ws.addEventListener('open', r, { once: true }));
  let id = 0; const pending = new Map();
  ws.addEventListener('message', (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) { const { res, rej } = pending.get(m.id); pending.delete(m.id); m.error ? rej(new Error(JSON.stringify(m.error))) : res(m.result); }
  });
  const send = (method, params = {}) => new Promise((res, rej) => { const i = ++id; pending.set(i, { res, rej }); ws.send(JSON.stringify({ id: i, method, params })); });
  await send('Page.enable'); await send('Runtime.enable');
  const b = {
    send,
    async size(w, h, mobile = false) { await send('Emulation.setDeviceMetricsOverride', { width: w, height: h, deviceScaleFactor: 1, mobile }); },
    async eval(expr) { const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || 'eval error'); return r.result.value; },
    async go(hash, wait = 1500) { await b.eval(`location.hash = ${JSON.stringify(hash)}`); await sleep(wait); },
    async open(url, wait = 2000) { await send('Page.navigate', { url }); await sleep(wait); },
    async waitFor(sel, timeout = 15000) { const t0 = Date.now(); while (Date.now() - t0 < timeout) { if (await b.eval(`!!document.querySelector(${JSON.stringify(sel)})`)) return true; await sleep(150); } throw new Error('timeout waiting for ' + sel); },
    async rect(sel) { return b.eval(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) return null; const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`); },
    async clickAt(x, y) {
      await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
      await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1, buttons: 1 });
      await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1, buttons: 0 });
    },
    async click(sel) { const r = await b.rect(sel); if (!r) throw new Error('no element ' + sel); await b.clickAt(r.x, r.y); },
    async clickText(sel, text) {
      const r = await b.eval(`(() => { const e = [...document.querySelectorAll(${JSON.stringify(sel)})].find(x => x.textContent.trim().includes(${JSON.stringify(text)}) && x.offsetParent); if (!e) return null; e.scrollIntoView({block:'center'}); const r = e.getBoundingClientRect(); return { x: r.x + r.width/2, y: r.y + r.height/2 }; })()`);
      if (!r) throw new Error(`no ${sel} with text ${text}`); await b.clickAt(r.x, r.y);
    },
    async square(sq) { return b.click(`.gm-sq[data-square="${sq}"]`); },
    async move(uci, gap = 250) { await b.square(uci.slice(0, 2)); await sleep(gap); await b.square(uci.slice(2, 4)); },
    async key(key, code = key) {
      const vk = { ArrowRight: 39, ArrowLeft: 37 }[key] || 0;
      await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode: vk });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: vk });
    },
    async shot(file) { const r = await send('Page.captureScreenshot', { format: 'png' }); writeFileSync(file, Buffer.from(r.data, 'base64')); },
    /** Record frames while `fn` runs, then encode a palette-optimized GIF with ffmpeg. */
    async record(file, fn, { fps = 8, width = 1100, tail = 1500 } = {}) {
      const dir = file + '.frames'; rmSync(dir, { recursive: true, force: true }); mkdirSync(dir, { recursive: true });
      let n = 0, on = true;
      const loop = (async () => {
        while (on) {
          const t = Date.now();
          const r = await send('Page.captureScreenshot', { format: 'png' });
          writeFileSync(path.join(dir, String(n++).padStart(5, '0') + '.png'), Buffer.from(r.data, 'base64'));
          await sleep(Math.max(0, 1000 / fps - (Date.now() - t)));
        }
      })();
      await fn(); await sleep(tail); on = false; await loop;
      const vf = `fps=${fps},scale=${width}:-1:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=192:stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle`;
      execFileSync('ffmpeg', ['-y', '-loglevel', 'error', '-framerate', String(fps), '-i', path.join(dir, '%05d.png'), '-vf', vf, '-loop', '0', file]);
      rmSync(dir, { recursive: true, force: true });
    },
    close() { try { ws.close(); } catch {} proc.kill(); },
  };
  return b;
}
