// Local page (#/local). Owned by the Local feature; see docs/CONTRACT.md.
import { h, disposables } from '../ui.js';
import { t } from '../i18n.js';

export const title = () => t('nav.routes.local');

export async function mount(root) {
  const bag = disposables();
  root.append(h('div', { class: 'page' }, h('h1', null, t('nav.routes.local'))));
  return () => bag.dispose();
}
