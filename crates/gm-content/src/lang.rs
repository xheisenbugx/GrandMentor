//! User-facing language. Shared by every crate that produces human text.
//!
//! Adding a language: add a variant, add it to [`Lang::ALL`] (and bump [`Lang::COUNT`]), fill
//! in [`Lang::code`] and [`Lang::name`], then follow the compiler: every message table either
//! `match`es on `Lang` exhaustively or is a `[_; Lang::COUNT]` array given to [`Lang::pick`],
//! so each one that needs the new language shows up as an error.

use serde::{Deserialize, Serialize};

/// A supported UI / content language. English is the default and the fallback.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    En,
    Es,
    /// Portuguese (Brazilian). `pt`, `pt-BR` and `pt-PT` all map here.
    Pt,
    Fr,
    De,
}

/// Longest `Accept-Language` header we bother to look at (bounded work on hostile input).
const MAX_ACCEPT_LANGUAGE: usize = 512;
/// Most language ranges considered from one header.
const MAX_RANGES: usize = 16;

impl Lang {
    /// Number of supported languages.
    pub const COUNT: usize = 5;

    /// Every supported language, default first.
    pub const ALL: [Lang; Lang::COUNT] = [Lang::En, Lang::Es, Lang::Pt, Lang::Fr, Lang::De];

    /// Pick this language's entry from a table in [`Lang::ALL`] order, `[en, es, pt, fr, de]`:
    /// `lang.pick(["Hello", "Hola", "Olá", "Bonjour", "Hallo"])`. Works for any entry type
    /// (strings, slices of alternatives, values). The one shared helper for inline message
    /// tables; a missing language is a compile error (the array must have `COUNT` entries).
    pub fn pick<T>(self, table: [T; Lang::COUNT]) -> T {
        let [en, es, pt, fr, de] = table;
        match self {
            Lang::En => en,
            Lang::Es => es,
            Lang::Pt => pt,
            Lang::Fr => fr,
            Lang::De => de,
        }
    }

    /// ISO 639-1 code, as used on the wire, in `?lang=` and in `data/i18n/<code>/`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
            Lang::Pt => "pt",
            Lang::Fr => "fr",
            Lang::De => "de",
        }
    }

    /// Native display name.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Es => "Español",
            Lang::Pt => "Português",
            Lang::Fr => "Français",
            Lang::De => "Deutsch",
        }
    }

    /// Parses a language tag such as `es`, `ES`, `es-MX`, `pt-BR` or `de_CH` by its primary subtag.
    /// Returns `None` for unsupported or malformed tags.
    pub fn parse(tag: &str) -> Option<Lang> {
        let tag = tag.trim();
        if tag.is_empty() || tag.len() > 35 {
            return None;
        }
        let primary = tag.split(['-', '_']).next().unwrap_or("");
        Lang::ALL.into_iter().find(|l| primary.eq_ignore_ascii_case(l.code()))
    }

    /// Picks the best supported language from an `Accept-Language` header value
    /// (e.g. `"es-MX,es;q=0.9,en;q=0.8"` → `Es`). Ranges are ordered by their `q` weight
    /// (stable for equal weights); `q=0` excludes a range; `*` matches the default. Returns
    /// `None` when nothing supported is acceptable.
    pub fn from_accept_language(header: &str) -> Option<Lang> {
        let header = match header.char_indices().nth(MAX_ACCEPT_LANGUAGE) {
            Some((i, _)) => &header[..i],
            None => header,
        };
        let mut ranges: Vec<(u16, usize, &str)> = Vec::new();
        for (i, part) in header.split(',').take(MAX_RANGES).enumerate() {
            let mut it = part.split(';');
            let tag = it.next().unwrap_or("").trim();
            if tag.is_empty() {
                continue;
            }
            let mut q: u16 = 1000;
            for param in it {
                let param = param.trim();
                if let Some(v) = param.strip_prefix("q=").or_else(|| param.strip_prefix("Q=")) {
                    q = match v.trim().parse::<f32>() {
                        Ok(f) if f.is_finite() => (f.clamp(0.0, 1.0) * 1000.0).round() as u16,
                        _ => 0,
                    };
                }
            }
            if q > 0 {
                ranges.push((q, i, tag));
            }
        }
        // Highest q first; header order breaks ties.
        ranges.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranges.into_iter().find_map(|(_, _, tag)| if tag == "*" { Some(Lang::default()) } else { Lang::parse(tag) })
    }

    /// `?lang=` value first (if supported), then the `Accept-Language` header, then English.
    pub fn negotiate(query_lang: Option<&str>, accept_language: Option<&str>) -> Lang {
        query_lang
            .and_then(Lang::parse)
            .or_else(|| accept_language.and_then(Lang::from_accept_language))
            .unwrap_or_default()
    }
}

impl std::fmt::Display for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::str::FromStr for Lang {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Lang::parse(s).ok_or_else(|| format!("unsupported language: {s:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_follows_all_order() {
        let table = ["en", "es", "pt", "fr", "de"];
        for lang in Lang::ALL {
            assert_eq!(lang.pick(table), lang.code());
        }
        let alternatives: [&[&str]; Lang::COUNT] = [&["a", "b"], &["c"], &[], &["d"], &["e"]];
        assert_eq!(Lang::En.pick(alternatives), &["a", "b"]);
        assert_eq!(Lang::De.pick([String::from("x"), "y".into(), "z".into(), "w".into(), "v".into()]), "v");
    }

    #[test]
    fn parse_tags() {
        assert_eq!(Lang::parse("es"), Some(Lang::Es));
        assert_eq!(Lang::parse("ES-mx"), Some(Lang::Es));
        assert_eq!(Lang::parse("es_ES"), Some(Lang::Es));
        assert_eq!(Lang::parse(" en-US "), Some(Lang::En));
        assert_eq!(Lang::parse("fr"), Some(Lang::Fr));
        assert_eq!(Lang::parse("fr-CA"), Some(Lang::Fr));
        assert_eq!(Lang::parse("pt"), Some(Lang::Pt));
        assert_eq!(Lang::parse("pt-BR"), Some(Lang::Pt));
        assert_eq!(Lang::parse("pt_PT"), Some(Lang::Pt));
        assert_eq!(Lang::parse("de"), Some(Lang::De));
        assert_eq!(Lang::parse("de-AT"), Some(Lang::De));
        assert_eq!(Lang::parse("DE-ch"), Some(Lang::De));
        assert_eq!(Lang::parse("it"), None);
        assert_eq!(Lang::parse("port"), None);
        assert_eq!(Lang::parse(""), None);
        assert_eq!(Lang::parse("espanol"), None);
        assert_eq!("es".parse::<Lang>(), Ok(Lang::Es));
        assert_eq!(Lang::default(), Lang::En);
    }

    #[test]
    fn accept_language() {
        assert_eq!(Lang::from_accept_language("es-MX,es;q=0.9,en;q=0.8"), Some(Lang::Es));
        assert_eq!(Lang::from_accept_language("en-US,en;q=0.9,es;q=0.8"), Some(Lang::En));
        assert_eq!(Lang::from_accept_language("fr-FR,fr;q=0.9,es;q=0.8,en;q=0.7"), Some(Lang::Fr));
        assert_eq!(Lang::from_accept_language("it-IT,it;q=0.9,es;q=0.8,en;q=0.7"), Some(Lang::Es));
        assert_eq!(Lang::from_accept_language("pt-BR,pt;q=0.9,en-US;q=0.8"), Some(Lang::Pt));
        assert_eq!(Lang::from_accept_language("de-CH, de;q=0.9, en;q=0.5"), Some(Lang::De));
        assert_eq!(Lang::from_accept_language("en;q=0.3, de-AT;q=0.7, fr-CA;q=0.8"), Some(Lang::Fr));
        assert_eq!(Lang::from_accept_language("ja, pt-PT;q=0.4"), Some(Lang::Pt));
        assert_eq!(Lang::from_accept_language("en;q=0.2, es;q=0.9"), Some(Lang::Es));
        assert_eq!(Lang::from_accept_language("es;q=0, en"), Some(Lang::En));
        assert_eq!(Lang::from_accept_language("it, nl"), None);
        assert_eq!(Lang::from_accept_language("it, *;q=0.5"), Some(Lang::En));
        assert_eq!(Lang::from_accept_language(""), None);
        assert_eq!(Lang::from_accept_language("es;q=abc, en;q=0.1"), Some(Lang::En));
        let long = "x,".repeat(10_000);
        assert_eq!(Lang::from_accept_language(&long), None);
    }

    #[test]
    fn negotiate_prefers_query() {
        assert_eq!(Lang::negotiate(Some("es"), Some("en")), Lang::Es);
        assert_eq!(Lang::negotiate(Some("xx"), Some("es-AR")), Lang::Es);
        assert_eq!(Lang::negotiate(None, None), Lang::En);
        assert_eq!(Lang::negotiate(Some("pt-BR"), Some("es")), Lang::Pt);
        assert_eq!(Lang::negotiate(Some("zz"), Some("de-DE,en;q=0.5")), Lang::De);
    }

    #[test]
    fn serde_snake_case() {
        assert_eq!(serde_json::to_string(&Lang::Es).unwrap(), "\"es\"");
        assert_eq!(serde_json::from_str::<Lang>("\"en\"").unwrap(), Lang::En);
        for l in Lang::ALL {
            let json = serde_json::to_string(&l).unwrap();
            assert_eq!(json, format!("\"{}\"", l.code()));
            assert_eq!(serde_json::from_str::<Lang>(&json).unwrap(), l);
            assert_eq!(Lang::parse(l.code()), Some(l));
            assert!(!l.name().is_empty());
        }
    }
}
