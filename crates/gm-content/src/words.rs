//! Chess vocabulary per language, with the grammatical information templates need.
//!
//! Spanish nouns carry a gender (la dama, la torre, el alfil, el caballo, el peón, el rey), and
//! articles, contractions (`al`, `del`), adjectives and pronouns must agree with it. Rather than
//! concatenating fragments, message templates use placeholders produced by [`PieceRef::vars`]:
//!
//! | key          | English              | Spanish (alfil)        | Spanish (dama)        |
//! |--------------|----------------------|------------------------|-----------------------|
//! | `{p}`        | the bishop on c4     | el alfil en c4         | la dama en d1         |
//! | `{P}`        | The bishop on c4     | El alfil en c4         | La dama en d1         |
//! | `{p_al}`     | the bishop on c4     | al alfil en c4         | a la dama en d1       |
//! | `{p_del}`    | the bishop on c4     | del alfil en c4        | de la dama en d1      |
//! | `{p_un}`     | a bishop             | un alfil               | una dama              |
//! | `{p_n}`      | bishop               | alfil                  | dama                  |
//! | `{p_ns}`     | bishops              | alfiles                | damas                 |
//! | `{p_o}`      | (empty)              | o  (colgad**o**)       | a  (colgad**a**)      |
//! | `{p_lo}`     | it                   | lo                     | la                    |
//!
//! (`p` is the prefix passed to [`PieceRef::vars`].)

use shakmaty::{Color, Role, Square};

use crate::Lang;

/// A Spanish noun with its gender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EsNoun {
    pub sing: &'static str,
    pub plural: &'static str,
    pub fem: bool,
}

impl EsNoun {
    /// Definite article: "el" / "la".
    pub fn el(self) -> &'static str {
        if self.fem {
            "la"
        } else {
            "el"
        }
    }
    /// "a" + article: "al" / "a la".
    pub fn al(self) -> &'static str {
        if self.fem {
            "a la"
        } else {
            "al"
        }
    }
    /// "de" + article: "del" / "de la".
    pub fn del(self) -> &'static str {
        if self.fem {
            "de la"
        } else {
            "del"
        }
    }
    /// Indefinite article: "un" / "una".
    pub fn un(self) -> &'static str {
        if self.fem {
            "una"
        } else {
            "un"
        }
    }
    /// Adjective / participle ending: "o" / "a".
    pub fn o(self) -> &'static str {
        if self.fem {
            "a"
        } else {
            "o"
        }
    }
    /// Direct-object pronoun: "lo" / "la".
    pub fn lo(self) -> &'static str {
        if self.fem {
            "la"
        } else {
            "lo"
        }
    }
}

/// Spanish noun for a piece (glossary: rey, dama, torre, alfil, caballo, peón).
pub fn es_piece(role: Role) -> EsNoun {
    match role {
        Role::Pawn => EsNoun { sing: "peón", plural: "peones", fem: false },
        Role::Knight => EsNoun { sing: "caballo", plural: "caballos", fem: false },
        Role::Bishop => EsNoun { sing: "alfil", plural: "alfiles", fem: false },
        Role::Rook => EsNoun { sing: "torre", plural: "torres", fem: true },
        Role::Queen => EsNoun { sing: "dama", plural: "damas", fem: true },
        Role::King => EsNoun { sing: "rey", plural: "reyes", fem: false },
    }
}

/// The generic word "piece" ("pieza", feminine).
pub const ES_PIECE: EsNoun = EsNoun { sing: "pieza", plural: "piezas", fem: true };

/// Bare piece name ("bishop" / "alfil").
pub fn piece_name(role: Role, lang: Lang) -> &'static str {
    match lang {
        Lang::En => match role {
            Role::Pawn => "pawn",
            Role::Knight => "knight",
            Role::Bishop => "bishop",
            Role::Rook => "rook",
            Role::Queen => "queen",
            Role::King => "king",
        },
        Lang::Es => es_piece(role).sing,
    }
}

/// Plural piece name ("bishops" / "alfiles").
pub fn piece_plural(role: Role, lang: Lang) -> &'static str {
    match lang {
        Lang::En => match role {
            Role::Pawn => "pawns",
            Role::Knight => "knights",
            Role::Bishop => "bishops",
            Role::Rook => "rooks",
            Role::Queen => "queens",
            Role::King => "kings",
        },
        Lang::Es => es_piece(role).plural,
    }
}

/// Side name as a sentence subject: "White" / "las blancas" (Spanish takes plural verbs).
pub fn side_name(c: Color, lang: Lang) -> &'static str {
    match (lang, c) {
        (Lang::En, Color::White) => "White",
        (Lang::En, Color::Black) => "Black",
        (Lang::Es, Color::White) => "las blancas",
        (Lang::Es, Color::Black) => "las negras",
    }
}

/// Spanish colour adjective agreeing with `noun`: "blanco" / "negra"...
pub fn es_color_adj(c: Color, noun: EsNoun) -> String {
    let stem = match c {
        Color::White => "blanc",
        Color::Black => "negr",
    };
    format!("{stem}{}", noun.o())
}

/// Uppercase the first character.
pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn en_article(word: &str) -> &'static str {
    match word.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}

/// A reference to a piece, optionally on a square and/or with its colour, rendered into
/// grammatical template variables (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PieceRef {
    pub role: Role,
    pub square: Option<Square>,
    /// When set, the phrase names the colour: "White's knight on d5" / "el caballo blanco en d5".
    pub color: Option<Color>,
}

impl PieceRef {
    pub fn new(role: Role) -> Self {
        PieceRef { role, square: None, color: None }
    }
    pub fn on(role: Role, square: Square) -> Self {
        PieceRef { role, square: Some(square), color: None }
    }
    /// The king is never qualified with its square ("the king" / "el rey").
    pub fn on_or_king(role: Role, square: Square) -> Self {
        if role == Role::King {
            PieceRef::new(role)
        } else {
            PieceRef::on(role, square)
        }
    }
    pub fn colored(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }

    /// Spanish noun phrase without article: "alfil blanco en c4".
    fn es_core(&self) -> String {
        let n = es_piece(self.role);
        let mut s = n.sing.to_string();
        if let Some(c) = self.color {
            s.push(' ');
            s.push_str(&es_color_adj(c, n));
        }
        if let Some(sq) = self.square {
            s.push_str(&format!(" en {sq}"));
        }
        s
    }

    /// Definite noun phrase: "the bishop on c4" / "el alfil en c4".
    pub fn def(&self, lang: Lang) -> String {
        match lang {
            Lang::En => {
                let name = piece_name(self.role, lang);
                let mut s = match self.color {
                    Some(c) => format!("{}'s {name}", side_name(c, lang)),
                    None => format!("the {name}"),
                };
                if let Some(sq) = self.square {
                    s.push_str(&format!(" on {sq}"));
                }
                s
            }
            Lang::Es => format!("{} {}", es_piece(self.role).el(), self.es_core()),
        }
    }

    /// Template variables for this piece under `prefix` (see module docs).
    pub fn vars(&self, prefix: &str, lang: Lang) -> Vec<(String, String)> {
        let def = self.def(lang);
        let k = |suffix: &str| if suffix.is_empty() { prefix.to_string() } else { format!("{prefix}_{suffix}") };
        let mut v = Vec::with_capacity(9);
        match lang {
            Lang::En => {
                let n = piece_name(self.role, lang);
                v.push((k("al"), def.clone()));
                v.push((k("del"), def.clone()));
                v.push((k("un"), format!("{} {n}", en_article(n))));
                v.push((k("n"), n.to_string()));
                v.push((k("ns"), piece_plural(self.role, lang).to_string()));
                v.push((k("o"), String::new()));
                v.push((k("lo"), "it".to_string()));
            }
            Lang::Es => {
                let n = es_piece(self.role);
                let core = self.es_core();
                v.push((k("al"), format!("{} {core}", n.al())));
                v.push((k("del"), format!("{} {core}", n.del())));
                v.push((k("un"), format!("{} {}", n.un(), n.sing)));
                v.push((k("n"), n.sing.to_string()));
                v.push((k("ns"), n.plural.to_string()));
                v.push((k("o"), n.o().to_string()));
                v.push((k("lo"), n.lo().to_string()));
            }
        }
        // Capitalized form uses the prefix uppercased: `{P}` for `p`, `{A}` for `a`.
        v.push((prefix.to_uppercase(), capitalize(&def)));
        v.push((k(""), def));
        v
    }
}

/// Fill `{key}` placeholders. Longer keys are substituted first so `{p_al}` is never
/// clobbered by `{p}`.
pub fn fill(template: &str, vars: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = vars.iter().collect();
    sorted.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    let mut out = template.to_string();
    for (k, v) in sorted {
        let needle = format!("{{{k}}}");
        if out.contains(&needle) {
            out = out.replace(&needle, v);
        }
    }
    out
}

/// Convenience to build owned template variables from string slices.
pub fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// Join a list naturally: "a, b and c" / "a, b y c".
pub fn join_list(items: &[String], lang: Lang) -> String {
    let and = match lang {
        Lang::En => "and",
        Lang::Es => "y",
    };
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} {and} {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

/// Format a number with one decimal in the language's convention ("92.3" / "92,3").
pub fn decimal1(v: f32, lang: Lang) -> String {
    let s = format!("{v:.1}");
    match lang {
        Lang::En => s,
        Lang::Es => s.replace('.', ","),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanish_agreement() {
        let bishop = PieceRef::on(Role::Bishop, Square::C4);
        let v = bishop.vars("p", Lang::Es);
        assert_eq!(fill("tu {p_n} en c4 queda colgad{p_o}", &v), "tu alfil en c4 queda colgado");
        assert_eq!(fill("ataca {p_al}", &v), "ataca al alfil en c4");
        let queen = PieceRef::new(Role::Queen);
        let v = queen.vars("p", Lang::Es);
        assert_eq!(fill("{P} queda colgad{p_o}; captúra{p_lo}", &v), "La dama queda colgada; captúrala");
        assert_eq!(fill("ataca {p_al}", &v), "ataca a la dama");
        let rook = PieceRef::on(Role::Rook, Square::A8).colored(Color::Black);
        assert_eq!(rook.def(Lang::Es), "la torre negra en a8");
        assert_eq!(rook.def(Lang::En), "Black's rook on a8");
        assert_eq!(PieceRef::on_or_king(Role::King, Square::E8).def(Lang::Es), "el rey");
        assert_eq!(fill("{p_un}", &PieceRef::new(Role::Knight).vars("p", Lang::En)), "a knight");
    }

    #[test]
    fn lists_and_numbers() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(join_list(&items, Lang::Es), "a, b y c");
        assert_eq!(join_list(&items[..1], Lang::En), "a");
        assert_eq!(decimal1(92.34, Lang::Es), "92,3");
    }
}
