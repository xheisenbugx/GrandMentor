// Insights page (#/insights). Owned by the Insights feature; see docs/CONTRACT.md.
import { h, disposables } from '../ui.js';
import { t } from '../i18n.js';

export const title = () => t('nav.routes.insights');

export async function mount(root) {
  const bag = disposables();
  root.append(h('div', { class: 'page' }, h('h1', null, t('nav.routes.insights'))));
  return () => bag.dispose();
}
