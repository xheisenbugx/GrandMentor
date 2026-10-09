// "Your weekly set" entry points outside the set itself:
//
//   const entry = weeklyHubEntry();       // puzzles hub banner { el, destroy }
//   const card = weeklyInsightsCard();    // Insights card { el, destroy }: this week + solve rate per theme
//   ensureWeeklyCss();                    // inject /css/weekly.css once
//
// Both read GET /api/weekly/history (never builds a set) and abort their fetch on destroy().
// See docs/CONTRACT.md "Weekly personal set".

import { api, isAbort } from '../api.js';
import { h, icon, escapeHtml } from '../ui.js';
import { t, hasKey, formatNumber, formatDateIntl } from '../i18n.js';

/** Weeks shown in the Insights card. */
const CARD_WEEKS = 4;
/** Themes shown in the Insights card (most practiced first). */
const CARD_THEMES = 5;

export function ensureWeeklyCss() {
  if (document.querySelector('link[data-page-css="weekly"]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/css/weekly.css';
  link.dataset.pageCss = 'weekly';
  document.head.appendChild(link);
}

function themeName(id) {
  const s = String(id || '');
  return s && hasKey(`themes.${s}`) ? t(`themes.${s}`) : s;
}

const span = (text) => `<span>${escapeHtml(text)}</span>`;

/** Puzzles hub banner linking to #/puzzles/weekly with this week's progress. */
export function weeklyHubEntry() {
  ensureWeeklyCss();
  const ctrl = new AbortController();
  const stats = h('div', { class: 'row-sm subtle text-sm wk-entry-stats' }, h('span', null, t('weekly.hub.notStarted')));
  const cta = h('span', { class: 'btn btn-secondary wk-entry-cta', html: icon('arrow-right') + span(t('weekly.hub.cta')) });
  const el = h('a', { class: 'card card-link wk-entry', href: '#/puzzles/weekly' },
    h('div', { class: 'pz-mode-icon wk-entry-icon', html: icon('calendar') }),
    h('div', { class: 'wk-entry-main' },
      h('span', { class: 'pz-mode-title' }, t('weekly.hub.title')),
      h('p', { class: 'muted' }, t('weekly.hub.desc')),
      stats),
    cta);
  api.get('/api/weekly/history?weeks=1', { signal: ctrl.signal }).then((d) => {
    const p = Array.isArray(d?.weeks) ? d.weeks[d.weeks.length - 1]?.progress : null;
    if (!p || !p.total) return;
    const finished = p.done >= p.total;
    stats.replaceChildren(
      h('span', { class: 'tabular' }, t('weekly.progress', { done: formatNumber(p.done), total: formatNumber(p.total) })),
      h('span', { class: 'dot-sep' }),
      h('span', null, t('weekly.solvedCount', { count: p.solved })));
    if (p.done > 0) {
      cta.className = `btn ${finished ? 'btn-ghost' : 'btn-primary'} wk-entry-cta`;
      cta.innerHTML = icon(finished ? 'check' : 'play') + span(t(finished ? 'weekly.hub.ctaDone' : 'weekly.hub.ctaContinue'));
    }
  }).catch((e) => { if (!isAbort(e)) { /* the banner still links to the set */ } });
  return { el, destroy() { ctrl.abort(); } };
}

function weekNum(key) {
  const m = /W(\d+)$/.exec(String(key || ''));
  return m ? String(Number(m[1])) : '';
}

/** Insights card: this week's progress and the solve rate per theme over the last weeks. */
export function weeklyInsightsCard() {
  ensureWeeklyCss();
  const ctrl = new AbortController();
  const body = h('div', { class: 'stack-sm' }, h('div', { class: 'skeleton skeleton-text' }), h('div', { class: 'skeleton skeleton-text', style: 'width:70%' }));
  const el = h('section', { class: 'card ins-card wk-ins' },
    h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('calendar') + span(t('weekly.card.title')) })),
    h('p', { class: 'muted text-sm ins-card-sub' }, t('weekly.card.subtitle')),
    body);

  api.get(`/api/weekly/history?weeks=${CARD_WEEKS}`, { signal: ctrl.signal }).then((d) => {
    const all = Array.isArray(d?.weeks) ? d.weeks : [];
    const current = all[all.length - 1];
    // Weeks without a set are skipped (this week always shows).
    const weeks = all.filter((w) => w.progress || w === current);
    const p = current?.progress;
    const pct = p?.total ? Math.round((p.done / p.total) * 100) : 0;
    const open = h('a', { class: 'btn btn-secondary btn-sm ins-card-link', href: '#/puzzles/weekly', html: icon('play') + span(t('weekly.card.open')) });
    const thisWeek = h('div', { class: 'stack-sm' },
      h('div', { class: 'row-sm' },
        h('span', { class: 'stat-label' }, t('weekly.card.thisWeek')),
        h('div', { class: 'spacer' }),
        h('span', { class: 'semibold tabular text-sm' }, p?.total ? t('weekly.progress', { done: formatNumber(p.done), total: formatNumber(p.total) }) : t('weekly.card.notStarted'))),
      h('div', { class: 'progress progress-sm' }, h('div', { class: 'progress-bar', style: { width: `${pct}%` } })));

    // Themes with any attempts, most practiced first.
    const totals = new Map();
    for (const w of weeks) for (const s of w.themes || []) totals.set(s.theme, (totals.get(s.theme) || 0) + (s.attempted | 0));
    const themes = [...totals.entries()].filter(([, n]) => n > 0).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, CARD_THEMES).map(([k]) => k);
    if (!themes.length) {
      body.replaceChildren(thisWeek, h('p', { class: 'ins-none' }, t('weekly.card.none')), open);
      return;
    }
    const cell = (w, theme) => (w.themes || []).find((s) => s.theme === theme && s.attempted > 0) || null;
    const head = h('div', { class: 'wk-tbl-row wk-tbl-head', role: 'row' },
      h('span', { class: 'wk-tbl-theme', role: 'columnheader' }, t('weekly.card.theme')),
      weeks.map((w) => {
        const d = new Date(`${w.week_start}T12:00:00Z`);
        const label = Number.isNaN(d.getTime()) ? w.week : formatDateIntl(d, { month: 'short', day: 'numeric' });
        return h('span', { class: 'wk-tbl-cell', role: 'columnheader', title: t('weekly.weekOf', { date: label }) }, t('weekly.card.weekShort', { week: weekNum(w.week) }));
      }),
      h('span', { class: 'wk-tbl-trend', role: 'columnheader', 'aria-hidden': 'true' }, ''));
    const rows = themes.map((theme) => {
      const rates = weeks.map((w) => {
        const c = cell(w, theme);
        return c ? Math.round((c.solved / c.attempted) * 100) : null;
      });
      const seen = rates.filter((r) => r != null);
      let trend = null;
      if (seen.length >= 2) {
        const diff = seen[seen.length - 1] - seen[seen.length - 2];
        trend = diff > 5 ? 'up' : diff < -5 ? 'down' : 'same';
      }
      const trendLabel = trend ? t(`weekly.card.trend${trend === 'up' ? 'Up' : trend === 'down' ? 'Down' : 'Same'}`) : '';
      return h('div', { class: 'wk-tbl-row', role: 'row' },
        h('span', { class: 'wk-tbl-theme truncate', role: 'rowheader', title: themeName(theme) }, themeName(theme)),
        rates.map((r, i) => {
          const c = cell(weeks[i], theme);
          return r == null
            ? h('span', { class: 'wk-tbl-cell is-empty', role: 'cell', title: t('weekly.card.noData') }, '·')
            : h('span', { class: 'wk-tbl-cell tabular', role: 'cell', style: { '--rate': `${r}%` }, title: `${c.solved}/${c.attempted}` }, `${r}%`);
        }),
        h('span', { class: `wk-tbl-trend${trend ? ` is-${trend}` : ''}`, role: 'cell', title: trendLabel || null, 'aria-label': trendLabel || null },
          trend ? (trend === 'up' ? '▲' : trend === 'down' ? '▼' : '•') : ''));
    });
    body.replaceChildren(thisWeek,
      h('div', { class: 'wk-tbl', role: 'table', style: { '--weeks': String(weeks.length) } }, head, rows),
      open);
  }).catch((e) => {
    if (isAbort(e)) return;
    el.remove();
  });
  return { el, destroy() { ctrl.abort(); } };
}
