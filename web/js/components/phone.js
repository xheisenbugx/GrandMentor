// "Use on your phone" section of the Settings page: phone access switch, LAN address + QR code,
// the security certificate (download + QR + Android/iOS steps), the access PIN and signed-in
// devices. Self-contained:
//
//   const section = createPhoneSection();
//   parent.appendChild(section.el);
//   ...
//   section.destroy();   // removes listeners, aborts fetches
//
// API: docs/CONTRACT.md "Phone & home use" (/api/phone/* answers only on this computer; other
// devices get 403 and see a short note plus "Sign out this device"). Guide: docs/PHONE.md.

import { api, isAbort } from '../api.js';
import { h, icon, disposables, toast, confirmDialog, copyText } from '../ui.js';
import { t } from '../i18n.js';

let cssPromise = null;
/** Load web/css/phone.css once. */
function ensurePhoneCss() {
  if (cssPromise) return cssPromise;
  const existing = document.querySelector('link[data-phone-css]');
  if (existing && existing.sheet) { cssPromise = Promise.resolve(); return cssPromise; }
  cssPromise = new Promise((resolve) => {
    const link = existing || document.createElement('link');
    let timer = 0;
    const done = () => { clearTimeout(timer); link.removeEventListener('load', done); link.removeEventListener('error', done); resolve(); };
    link.addEventListener('load', done);
    link.addEventListener('error', done);
    timer = setTimeout(done, 1500);
    if (!existing) {
      link.rel = 'stylesheet';
      link.href = '/css/phone.css';
      link.dataset.phoneCss = '1';
      document.head.appendChild(link);
    }
  });
  return cssPromise;
}

/** "123456" → "123 456". */
const spacedPin = (pin) => String(pin || '').replace(/^(\d{3})(\d{3})$/, '$1 $2');

/** QR image for one of the addresses the server advertised. */
function qrImage(url) {
  return h('div', { class: 'ph-qr' },
    h('img', { src: `/api/phone/qr?url=${encodeURIComponent(url)}`, alt: t('phone.qrAlt', { url }), width: '176', height: '176', decoding: 'async' }));
}

function stepList(items) {
  return h('ol', { class: 'ph-steps-list' }, (Array.isArray(items) ? items : [items]).map((s) => h('li', null, s)));
}

export function createPhoneSection() {
  ensurePhoneCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;
  // Listeners of the current render; replaced on every re-render.
  let rb = disposables();
  bag.add(() => rb.dispose());
  const resetRender = () => { rb.dispose(); rb = disposables(); };

  const body = h('div', { class: 'ph-body', 'aria-live': 'polite' }, h('p', { class: 'setting-row-desc' }, t('phone.loading')));
  const badge = h('span', { class: 'badge', hidden: true });
  let addrIndex = 0;

  const fill = (...kids) => body.replaceChildren(...kids.flat().filter((k) => k != null && k !== false));

  // ---- This device is not the computer running GrandMentor ---------------------------------
  const renderRemote = () => {
    resetRender();
    badge.hidden = true;
    const btn = h('button', { type: 'button', class: 'btn btn-secondary', html: icon('lock', { size: 16 }) + `<span>${t('phone.remote.signOut')}</span>` });
    rb.on(btn, 'click', async () => {
      btn.classList.add('loading');
      try { await api.post('/api/access/logout', {}, { signal }); } catch (e) { if (isAbort(e)) return; }
      location.assign('/login');
    });
    fill(h('div', { class: 'callout' }, h('span', { class: 'ph-ci', html: icon('info') }), h('div', { class: 'text-sm' }, t('phone.remote.text'))),
      h('div', { class: 'ph-actions' }, btn));
  };

  // ---- Toggle -------------------------------------------------------------------------------
  const toggleRow = (st) => {
    const input = h('input', { type: 'checkbox', 'aria-label': t('phone.toggle.aria'), checked: !!st.lan.enabled, disabled: !st.lan.can_save });
    rb.on(input, 'change', async () => {
      input.disabled = true;
      try {
        const next = await api.put('/api/phone/lan', { enabled: input.checked }, { signal });
        if (!bag.disposed) render(next);
      } catch (e) {
        if (isAbort(e) || bag.disposed) return;
        input.checked = !input.checked;
        input.disabled = false;
        toast(e.message, 'error');
      }
    });
    return h('div', { class: 'setting-row' },
      h('div', { class: 'setting-row-text' },
        h('div', { class: 'setting-row-title' }, t('phone.toggle.title')),
        h('div', { class: 'setting-row-desc' }, t('phone.toggle.desc')),
        st.lan.env != null ? h('div', { class: 'setting-row-desc subtle text-xs' }, t('phone.toggle.env')) : null),
      h('label', { class: 'switch' }, input, h('span', { class: 'switch-track' })));
  };

  // ---- Address picker (several network cards) ----------------------------------------------
  const addressPicker = (st) => {
    if (!Array.isArray(st.addresses) || st.addresses.length < 2) return null;
    const seg = h('div', { class: 'segmented ph-addr-seg', role: 'radiogroup', 'aria-label': t('phone.pickAddress') },
      st.addresses.map((a, i) => h('button', { type: 'button', role: 'radio', dataset: { i: String(i) }, class: i === addrIndex ? 'active' : null, 'aria-checked': String(i === addrIndex) }, a)));
    rb.on(seg, 'click', (e) => {
      const b = e.target.closest('button[data-i]');
      if (!b) return;
      addrIndex = Number(b.dataset.i) || 0;
      render(st);
    });
    return h('div', { class: 'ph-addr' }, h('span', { class: 'setting-row-title' }, t('phone.pickAddress')), seg);
  };

  const urlLine = (url) => {
    const copyBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-sm', html: icon('copy', { size: 16 }) + `<span>${t('phone.copy')}</span>` });
    rb.on(copyBtn, 'click', () => copyText(url, t('phone.copied')));
    return h('div', { class: 'ph-url' }, h('code', null, url), copyBtn);
  };

  // ---- PIN + devices ------------------------------------------------------------------------
  const pinBlock = (st) => {
    const pinEl = h('div', { class: 'ph-pin', role: 'text', 'aria-label': t('phone.step3.pinAria', { pin: String(st.pin || '').split('').join(' ') }) }, spacedPin(st.pin));
    const newBtn = h('button', { type: 'button', class: 'btn btn-secondary btn-sm', html: icon('refresh', { size: 16 }) + `<span>${t('phone.step3.newPin')}</span>` });
    rb.on(newBtn, 'click', async () => {
      newBtn.classList.add('loading');
      try {
        const r = await api.post('/api/phone/pin', {}, { signal });
        if (bag.disposed) return;
        pinEl.textContent = spacedPin(r.pin);
        pinEl.setAttribute('aria-label', t('phone.step3.pinAria', { pin: String(r.pin).split('').join(' ') }));
        toast(t('phone.step3.newPinDone'), 'success');
      } catch (e) {
        if (!isAbort(e)) toast(e.message, 'error');
      } finally {
        newBtn.classList.remove('loading');
      }
    });
    return h('div', { class: 'ph-pin-row' }, pinEl, newBtn);
  };

  const devicesRow = (st) => {
    const count = h('div', { class: 'setting-row-desc' }, t('phone.devices.count', { count: st.devices || 0 }));
    const btn = h('button', { type: 'button', class: 'btn btn-secondary', disabled: !st.devices, html: icon('users', { size: 16 }) + `<span>${t('phone.devices.signOutAll')}</span>` });
    rb.on(btn, 'click', async () => {
      const ok = await confirmDialog({ title: t('phone.devices.confirmTitle'), message: t('phone.devices.confirmText'), confirmLabel: t('phone.devices.signOutAll'), danger: true });
      if (!ok || bag.disposed) return;
      try {
        await api.del('/api/phone/devices', { signal });
        if (bag.disposed) return;
        count.textContent = t('phone.devices.count', { count: 0 });
        btn.disabled = true;
        toast(t('phone.devices.done'), 'success');
      } catch (e) {
        if (!isAbort(e)) toast(e.message, 'error');
      }
    });
    return h('div', { class: 'setting-row ph-devices' },
      h('div', { class: 'setting-row-text' }, h('div', { class: 'setting-row-title' }, t('phone.devices.title')), count),
      btn);
  };

  // ---- Main render --------------------------------------------------------------------------
  function render(st) {
    resetRender();
    const running = st.lan.running;
    const plain = !running && st.network_visible;
    badge.hidden = false;
    badge.className = `badge ${running ? 'badge-success' : plain ? 'badge-warning' : ''}`;
    badge.textContent = running ? t('phone.state.on') : plain ? t('phone.state.plain') : t('phone.state.off');

    const parts = [toggleRow(st)];
    if (st.lan.restart_required) {
      parts.push(h('div', { class: 'callout callout-warning' }, h('span', { class: 'ph-ci', html: icon('refresh') }),
        h('div', { class: 'text-sm' }, st.lan.enabled ? t('phone.restart.on') : t('phone.restart.off'))));
    }
    const apps = Array.isArray(st.app_urls) ? st.app_urls : [];
    const cas = Array.isArray(st.ca_urls) ? st.ca_urls : [];
    if ((running || plain) && apps.length === 0) {
      parts.push(h('div', { class: 'callout callout-warning' }, h('span', { class: 'ph-ci', html: icon('wifi-off') }), h('div', { class: 'text-sm' }, t('phone.noNetwork'))));
    }
    if ((running || plain) && apps.length) {
      if (addrIndex >= apps.length) addrIndex = 0;
      const appUrl = apps[addrIndex];
      parts.push(addressPicker(st));
      if (plain) {
        parts.push(h('div', { class: 'callout' }, h('span', { class: 'ph-ci', html: icon('info') }), h('div', { class: 'text-sm' }, t('phone.plainNote'))));
      }
      const steps = h('ol', { class: 'ph-steps' });
      if (running && cas.length) {
        const caUrl = cas[Math.min(addrIndex, cas.length - 1)];
        steps.appendChild(h('li', { class: 'ph-step' },
          h('div', { class: 'ph-step-text' },
            h('h3', { class: 'ph-step-title' }, t('phone.step1.title')),
            h('p', { class: 'setting-row-desc' }, t('phone.step1.text')),
            urlLine(caUrl),
            h('div', { class: 'ph-actions' },
              h('a', { class: 'btn btn-secondary btn-sm', href: st.ca?.download || '/phone/ca.crt', download: 'grandmentor-ca.crt', html: icon('download', { size: 16 }) + `<span>${t('phone.step1.download')}</span>` })),
            h('details', { class: 'ph-details' }, h('summary', null, t('phone.step1.androidTitle')), stepList(t('phone.step1.android'))),
            h('details', { class: 'ph-details' }, h('summary', null, t('phone.step1.iosTitle')), stepList(t('phone.step1.ios'))),
            st.ca?.fingerprint ? h('p', { class: 'subtle text-xs ph-fp' }, t('phone.step1.fingerprint', { value: st.ca.fingerprint })) : null),
          qrImage(caUrl)));
      }
      steps.appendChild(h('li', { class: 'ph-step' },
        h('div', { class: 'ph-step-text' },
          h('h3', { class: 'ph-step-title' }, t('phone.step2.title')),
          h('p', { class: 'setting-row-desc' }, t('phone.step2.text')),
          urlLine(appUrl),
          running ? h('p', { class: 'setting-row-desc subtle text-xs' }, t('phone.step2.install')) : null),
        qrImage(appUrl)));
      if (st.pin_required) {
        steps.appendChild(h('li', { class: 'ph-step' },
          h('div', { class: 'ph-step-text' },
            h('h3', { class: 'ph-step-title' }, t('phone.step3.title')),
            h('p', { class: 'setting-row-desc' }, t('phone.step3.text')),
            pinBlock(st))));
      }
      parts.push(steps);
      if (st.pin_required) parts.push(devicesRow(st));
      parts.push(h('p', { class: 'subtle text-xs ph-safety', html: icon('shield', { size: 14 }) + `<span>${t('phone.safety')}</span>` }));
    } else if (!running && !plain && !st.lan.restart_required) {
      parts.push(h('p', { class: 'setting-row-desc' }, t('phone.off.text')));
    }
    fill(parts);
  }

  const load = async () => {
    try {
      const st = await api.get('/api/phone/status', { signal });
      if (!bag.disposed) render(st);
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      if (e.status === 403) renderRemote();
      else fill(h('p', { class: 'setting-row-desc' }, t('phone.unavailable')));
    }
  };
  load();

  const el = h('section', { class: 'card ph-section', id: 'phone', 'aria-labelledby': 'settings-phone-title' },
    h('div', { class: 'card-header' },
      h('h2', { class: 'card-title', id: 'settings-phone-title', html: icon('wifi') + `<span>${t('phone.title')}</span>` }),
      badge),
    h('p', { class: 'setting-row-desc ph-intro' }, t('phone.intro')),
    body);

  return { el, destroy: bag.dispose };
}
