//! User-facing language. Shared by every crate that produces human text.
//!
//! Adding a language: add a variant, add it to [`Lang::ALL`], fill in [`Lang::code`] and
//! [`Lang::name`], then follow the compiler: every message table `match`es on `Lang`
//! exhaustively, so each one that needs the new language shows up as an error.

use serde::{Deserialize, Serialize};

/// A supported UI / content language. English is the default and the fallback.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    En,
    Es,
}

/// Longest `Accept-Language` header we bother to look at (bounded work on hostile input).
const MAX_ACCEPT_LANGUAGE: usize = 512;
/// Most language ranges considered from one header.
const MAX_RANGES: usize = 16;

impl Lang {
    /// Every supported language, default first.
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Es];

    /// ISO 639-1 code, as used on the wire, in `?lang=` and in `data/i18n/<code>/`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
        }
    }

    /// Native display name.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Es => "Español",
        }
    }

    /// Parses a language tag such as `es`, `ES`, `es-MX` or `es_ES` by its primary subtag.
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
    fn parse_tags() {
        assert_eq!(Lang::parse("es"), Some(Lang::Es));
        assert_eq!(Lang::parse("ES-mx"), Some(Lang::Es));
        assert_eq!(Lang::parse("es_ES"), Some(Lang::Es));
        assert_eq!(Lang::parse(" en-US "), Some(Lang::En));
        assert_eq!(Lang::parse("fr"), None);
        assert_eq!(Lang::parse(""), None);
        assert_eq!(Lang::parse("espanol"), None);
        assert_eq!("es".parse::<Lang>(), Ok(Lang::Es));
        assert_eq!(Lang::default(), Lang::En);
    }

    #[test]
    fn accept_language() {
        assert_eq!(Lang::from_accept_language("es-MX,es;q=0.9,en;q=0.8"), Some(Lang::Es));
        assert_eq!(Lang::from_accept_language("en-US,en;q=0.9,es;q=0.8"), Some(Lang::En));
        assert_eq!(Lang::from_accept_language("fr-FR,fr;q=0.9,es;q=0.8,en;q=0.7"), Some(Lang::Es));
        assert_eq!(Lang::from_accept_language("en;q=0.2, es;q=0.9"), Some(Lang::Es));
        assert_eq!(Lang::from_accept_language("es;q=0, en"), Some(Lang::En));
        assert_eq!(Lang::from_accept_language("fr, de"), None);
        assert_eq!(Lang::from_accept_language("fr, *;q=0.5"), Some(Lang::En));
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
    }

    #[test]
    fn serde_snake_case() {
        assert_eq!(serde_json::to_string(&Lang::Es).unwrap(), "\"es\"");
        assert_eq!(serde_json::from_str::<Lang>("\"en\"").unwrap(), Lang::En);
    }
}
