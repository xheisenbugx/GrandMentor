// GrandMentor — supported UI languages.
// To add a language: add an entry here, create web/locales/<code>/ (copy en/ and translate),
// and optionally data/i18n/<code>/ for lesson/opening/endgame content. See docs/I18N.md.

export const LANGUAGES = Object.freeze({
  en: Object.freeze({ name: 'English', nativeName: 'English', locale: 'en-US', dir: 'ltr' }),
  es: Object.freeze({ name: 'Spanish', nativeName: 'Español', locale: 'es-ES', dir: 'ltr' }),
  pt: Object.freeze({ name: 'Portuguese', nativeName: 'Português', locale: 'pt-BR', dir: 'ltr' }),
  fr: Object.freeze({ name: 'French', nativeName: 'Français', locale: 'fr-FR', dir: 'ltr' }),
  de: Object.freeze({ name: 'German', nativeName: 'Deutsch', locale: 'de-DE', dir: 'ltr' }),
});

export const DEFAULT_LANGUAGE = 'en';

/** True when `code` is a supported language code. */
export function isLanguage(code) {
  return typeof code === 'string' && Object.hasOwn(LANGUAGES, code);
}
