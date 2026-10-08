//! Chess vocabulary per language, with the grammatical information templates need.
//!
//! Romance nouns carry a gender (es: la dama, el alfil; pt: a dama, o bispo; fr: la dame, le
//! fou) and German nouns a gender *and* a case (der Läufer / den Läufer / dem Läufer / des
//! Läufers; der Bauer / den Bauern). Articles, contractions, adjectives and pronouns must agree.
//! Rather than concatenating fragments, message templates use placeholders produced by
//! [`PieceRef::vars`]. Templates are written per language, so each language only uses the keys
//! that make sense for it; every key is always present (falling back to the plain definite
//! phrase) so a template never shows a raw `{...}`.
//!
//! | key          | English          | Spanish             | Portuguese           | French               | German                    |
//! |--------------|------------------|---------------------|----------------------|----------------------|---------------------------|
//! | `{p}`        | the bishop on c4 | el alfil en c4      | o bispo em c4        | le fou en c4         | der Läufer auf c4 (nom.)  |
//! | `{P}`        | The bishop on c4 | El alfil en c4      | O bispo em c4        | Le fou en c4         | Der Läufer auf c4         |
//! | `{p_al}`     | the bishop on c4 | al alfil / a la dama| ao bispo / à dama    | au fou / à la dame   | dem Läufer auf c4 (dat.)  |
//! | `{p_del}`    | the bishop on c4 | del alfil / de la dama | do bispo / da dama | du fou / de la dame | des Läufers auf c4 (gen.) |
//! | `{p_acc}`    | the bishop on c4 | = `{p}`             | = `{p}`              | = `{p}`              | den Läufer auf c4 (acc.)  |
//! | `{p_un}`     | a bishop         | un alfil / una dama | um bispo / uma dama  | un fou / une dame    | ein Läufer / eine Dame    |
//! | `{p_un_acc}` | a bishop         | = `{p_un}`          | = `{p_un}`           | = `{p_un}`           | einen Läufer / eine Dame  |
//! | `{p_n}`      | bishop           | alfil               | bispo                | fou                  | Läufer                    |
//! | `{p_ns}`     | bishops          | alfiles             | bispos               | fous                 | Läufer (plural)           |
//! | `{p_o}`      | (empty)          | o / a (colgado/a)   | o / a (also the article: `n{p_o}` = no/na) | (empty) / e (attaqué/e) | (empty) |
//! | `{p_lo}`     | it               | lo / la             | o / a                | le / la              | ihn / sie (acc.)          |
//! | `{p_er}`     | it               | él / ella           | ele / ela            | il / elle            | er / sie (nom.)           |
//!
//! (`p` is the prefix passed to [`PieceRef::vars`]; `{P}` is the prefix uppercased.)

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

/// A gendered Portuguese or French noun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RomNoun {
    pub sing: &'static str,
    pub plural: &'static str,
    pub fem: bool,
}

/// Brazilian Portuguese noun for a piece (rei, dama, torre, bispo, cavalo, peão).
pub fn pt_piece(role: Role) -> RomNoun {
    match role {
        Role::Pawn => RomNoun { sing: "peão", plural: "peões", fem: false },
        Role::Knight => RomNoun { sing: "cavalo", plural: "cavalos", fem: false },
        Role::Bishop => RomNoun { sing: "bispo", plural: "bispos", fem: false },
        Role::Rook => RomNoun { sing: "torre", plural: "torres", fem: true },
        Role::Queen => RomNoun { sing: "dama", plural: "damas", fem: true },
        Role::King => RomNoun { sing: "rei", plural: "reis", fem: false },
    }
}

/// French noun for a piece (roi, dame, tour, fou, cavalier, pion). None starts with a vowel,
/// so the articles never elide.
pub fn fr_piece(role: Role) -> RomNoun {
    match role {
        Role::Pawn => RomNoun { sing: "pion", plural: "pions", fem: false },
        Role::Knight => RomNoun { sing: "cavalier", plural: "cavaliers", fem: false },
        Role::Bishop => RomNoun { sing: "fou", plural: "fous", fem: false },
        Role::Rook => RomNoun { sing: "tour", plural: "tours", fem: true },
        Role::Queen => RomNoun { sing: "dame", plural: "dames", fem: true },
        Role::King => RomNoun { sing: "roi", plural: "rois", fem: false },
    }
}

/// A German noun: every piece is masculine except die Dame. `Bauer` is a weak noun
/// (den/dem/des Bauern), so the oblique singular is stored separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeNoun {
    /// Nominative singular: "Läufer", "Bauer".
    pub sing: &'static str,
    /// Accusative/dative singular: "Läufer", "Bauern".
    pub obl: &'static str,
    /// Genitive singular: "Läufers", "Bauern", "Dame".
    pub gen: &'static str,
    pub plural: &'static str,
    pub fem: bool,
}

/// German noun for a piece (König, Dame, Turm, Läufer, Springer, Bauer).
pub fn de_piece(role: Role) -> DeNoun {
    match role {
        Role::Pawn => DeNoun { sing: "Bauer", obl: "Bauern", gen: "Bauern", plural: "Bauern", fem: false },
        Role::Knight => DeNoun { sing: "Springer", obl: "Springer", gen: "Springers", plural: "Springer", fem: false },
        Role::Bishop => DeNoun { sing: "Läufer", obl: "Läufer", gen: "Läufers", plural: "Läufer", fem: false },
        Role::Rook => DeNoun { sing: "Turm", obl: "Turm", gen: "Turms", plural: "Türme", fem: false },
        Role::Queen => DeNoun { sing: "Dame", obl: "Dame", gen: "Dame", plural: "Damen", fem: true },
        Role::King => DeNoun { sing: "König", obl: "König", gen: "Königs", plural: "Könige", fem: false },
    }
}

/// German grammatical case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeCase {
    Nom,
    Acc,
    Dat,
    Gen,
}

impl DeNoun {
    /// Definite article in `case`.
    pub fn article(self, case: DeCase) -> &'static str {
        match (self.fem, case) {
            (false, DeCase::Nom) => "der",
            (false, DeCase::Acc) => "den",
            (false, DeCase::Dat) => "dem",
            (false, DeCase::Gen) => "des",
            (true, DeCase::Nom | DeCase::Acc) => "die",
            (true, DeCase::Dat | DeCase::Gen) => "der",
        }
    }
    /// Noun form in `case`.
    pub fn form(self, case: DeCase) -> &'static str {
        match case {
            DeCase::Nom => self.sing,
            DeCase::Acc | DeCase::Dat => self.obl,
            DeCase::Gen => self.gen,
        }
    }
    /// Weak adjective ending after a definite article: "der weiße Läufer", "den weißen Läufer".
    pub fn weak_ending(self, case: DeCase) -> &'static str {
        match (self.fem, case) {
            (_, DeCase::Nom) | (true, DeCase::Acc) => "e",
            _ => "en",
        }
    }
}

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
        Lang::Pt => pt_piece(role).sing,
        Lang::Fr => fr_piece(role).sing,
        Lang::De => de_piece(role).sing,
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
        Lang::Pt => pt_piece(role).plural,
        Lang::Fr => fr_piece(role).plural,
        Lang::De => de_piece(role).plural,
    }
}

/// Side name as a sentence subject: "White" / "las blancas" / "as brancas" / "les Blancs" /
/// "Weiß". Spanish, Portuguese and French take plural verbs; German "Weiß" is singular.
pub fn side_name(c: Color, lang: Lang) -> &'static str {
    match (lang, c) {
        (Lang::En, Color::White) => "White",
        (Lang::En, Color::Black) => "Black",
        (Lang::Es, Color::White) => "las blancas",
        (Lang::Es, Color::Black) => "las negras",
        (Lang::Pt, Color::White) => "as brancas",
        (Lang::Pt, Color::Black) => "as pretas",
        (Lang::Fr, Color::White) => "les Blancs",
        (Lang::Fr, Color::Black) => "les Noirs",
        (Lang::De, Color::White) => "Weiß",
        (Lang::De, Color::Black) => "Schwarz",
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

/// Portuguese colour adjective: "branco" / "preta"...
pub fn pt_color_adj(c: Color, noun: RomNoun) -> &'static str {
    match (c, noun.fem) {
        (Color::White, false) => "branco",
        (Color::White, true) => "branca",
        (Color::Black, false) => "preto",
        (Color::Black, true) => "preta",
    }
}

/// French colour adjective: "blanc" / "blanche" / "noir" / "noire".
pub fn fr_color_adj(c: Color, noun: RomNoun) -> &'static str {
    match (c, noun.fem) {
        (Color::White, false) => "blanc",
        (Color::White, true) => "blanche",
        (Color::Black, false) => "noir",
        (Color::Black, true) => "noire",
    }
}

/// German colour adjective stem ("weiß" / "schwarz"), to be followed by an ending.
pub fn de_color_stem(c: Color) -> &'static str {
    match c {
        Color::White => "weiß",
        Color::Black => "schwarz",
    }
}

/// Plural choice by CLDR rules for cardinal `n`: French treats 0 and 1 as singular, the
/// other languages only 1.
pub fn plural<'a>(n: i64, one: &'a str, other: &'a str, lang: Lang) -> &'a str {
    let singular = match lang {
        Lang::Fr => (-1..=1).contains(&n),
        Lang::En | Lang::Es | Lang::Pt | Lang::De => n == 1 || n == -1,
    };
    if singular {
        one
    } else {
        other
    }
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

    /// Portuguese noun phrase without article: "cavalo branco em d5".
    fn pt_core(&self) -> String {
        let n = pt_piece(self.role);
        let mut s = n.sing.to_string();
        if let Some(c) = self.color {
            s.push(' ');
            s.push_str(pt_color_adj(c, n));
        }
        if let Some(sq) = self.square {
            s.push_str(&format!(" em {sq}"));
        }
        s
    }

    /// French noun phrase without article: "cavalier blanc en d5".
    fn fr_core(&self) -> String {
        let n = fr_piece(self.role);
        let mut s = n.sing.to_string();
        if let Some(c) = self.color {
            s.push(' ');
            s.push_str(fr_color_adj(c, n));
        }
        if let Some(sq) = self.square {
            s.push_str(&format!(" en {sq}"));
        }
        s
    }

    /// German definite noun phrase in `case`: "den weißen Springer auf d5".
    pub fn de_def(&self, case: DeCase) -> String {
        let n = de_piece(self.role);
        let mut s = n.article(case).to_string();
        if let Some(c) = self.color {
            s.push(' ');
            s.push_str(de_color_stem(c));
            s.push_str(n.weak_ending(case));
        }
        s.push(' ');
        s.push_str(n.form(case));
        if let Some(sq) = self.square {
            s.push_str(&format!(" auf {sq}"));
        }
        s
    }

    /// Definite noun phrase: "the bishop on c4" / "el alfil en c4" / "der Läufer auf c4".
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
            Lang::Pt => format!("{} {}", if pt_piece(self.role).fem { "a" } else { "o" }, self.pt_core()),
            Lang::Fr => format!("{} {}", if fr_piece(self.role).fem { "la" } else { "le" }, self.fr_core()),
            Lang::De => self.de_def(DeCase::Nom),
        }
    }

    /// Template variables for this piece under `prefix` (see module docs).
    pub fn vars(&self, prefix: &str, lang: Lang) -> Vec<(String, String)> {
        let def = self.def(lang);
        let k = |suffix: &str| if suffix.is_empty() { prefix.to_string() } else { format!("{prefix}_{suffix}") };
        let mut v = Vec::with_capacity(14);
        match lang {
            Lang::En => {
                let n = piece_name(self.role, lang);
                let un = format!("{} {n}", en_article(n));
                v.push((k("al"), def.clone()));
                v.push((k("del"), def.clone()));
                v.push((k("acc"), def.clone()));
                v.push((k("un_acc"), un.clone()));
                v.push((k("un"), un));
                v.push((k("n"), n.to_string()));
                v.push((k("ns"), piece_plural(self.role, lang).to_string()));
                v.push((k("o"), String::new()));
                v.push((k("lo"), "it".to_string()));
                v.push((k("er"), "it".to_string()));
            }
            Lang::Es => {
                let n = es_piece(self.role);
                let core = self.es_core();
                let un = format!("{} {}", n.un(), n.sing);
                v.push((k("al"), format!("{} {core}", n.al())));
                v.push((k("del"), format!("{} {core}", n.del())));
                v.push((k("acc"), def.clone()));
                v.push((k("un_acc"), un.clone()));
                v.push((k("un"), un));
                v.push((k("n"), n.sing.to_string()));
                v.push((k("ns"), n.plural.to_string()));
                v.push((k("o"), n.o().to_string()));
                v.push((k("lo"), n.lo().to_string()));
                v.push((k("er"), if n.fem { "ella" } else { "él" }.to_string()));
            }
            Lang::Pt => {
                let n = pt_piece(self.role);
                let core = self.pt_core();
                let un = format!("{} {}", if n.fem { "uma" } else { "um" }, n.sing);
                let o = if n.fem { "a" } else { "o" };
                v.push((k("al"), format!("{} {core}", if n.fem { "à" } else { "ao" })));
                v.push((k("del"), format!("d{o} {core}")));
                v.push((k("acc"), def.clone()));
                v.push((k("un_acc"), un.clone()));
                v.push((k("un"), un));
                v.push((k("n"), n.sing.to_string()));
                v.push((k("ns"), n.plural.to_string()));
                v.push((k("o"), o.to_string()));
                v.push((k("lo"), o.to_string()));
                v.push((k("er"), if n.fem { "ela" } else { "ele" }.to_string()));
            }
            Lang::Fr => {
                let n = fr_piece(self.role);
                let core = self.fr_core();
                let un = format!("{} {}", if n.fem { "une" } else { "un" }, n.sing);
                v.push((k("al"), format!("{} {core}", if n.fem { "à la" } else { "au" })));
                v.push((k("del"), format!("{} {core}", if n.fem { "de la" } else { "du" })));
                v.push((k("acc"), def.clone()));
                v.push((k("un_acc"), un.clone()));
                v.push((k("un"), un));
                v.push((k("n"), n.sing.to_string()));
                v.push((k("ns"), n.plural.to_string()));
                v.push((k("o"), if n.fem { "e" } else { "" }.to_string()));
                v.push((k("lo"), if n.fem { "la" } else { "le" }.to_string()));
                v.push((k("er"), if n.fem { "elle" } else { "il" }.to_string()));
            }
            Lang::De => {
                let n = de_piece(self.role);
                v.push((k("al"), self.de_def(DeCase::Dat)));
                v.push((k("del"), self.de_def(DeCase::Gen)));
                v.push((k("acc"), self.de_def(DeCase::Acc)));
                v.push((k("un"), format!("{} {}", if n.fem { "eine" } else { "ein" }, n.sing)));
                v.push((k("un_acc"), format!("{} {}", if n.fem { "eine" } else { "einen" }, n.obl)));
                v.push((k("n"), n.sing.to_string()));
                v.push((k("ns"), n.plural.to_string()));
                v.push((k("o"), String::new()));
                v.push((k("lo"), if n.fem { "sie" } else { "ihn" }.to_string()));
                v.push((k("er"), if n.fem { "sie" } else { "er" }.to_string()));
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
        Lang::Pt => "e",
        Lang::Fr => "et",
        Lang::De => "und",
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
        Lang::Es | Lang::Pt | Lang::Fr | Lang::De => s.replace('.', ","),
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
    fn portuguese_agreement() {
        let bishop = PieceRef::on(Role::Bishop, Square::C4);
        let v = bishop.vars("p", Lang::Pt);
        assert_eq!(fill("{P} ficou pendurad{p_o}; volte {p_al}", &v), "O bispo em c4 ficou pendurado; volte ao bispo em c4");
        let queen = PieceRef::new(Role::Queen);
        let v = queen.vars("p", Lang::Pt);
        assert_eq!(fill("crava {p}, n{p_o} {p_n} e {p_del}; {p_un}", &v), "crava a dama, na dama e da dama; uma dama");
        let rook = PieceRef::on(Role::Rook, Square::A8).colored(Color::Black);
        assert_eq!(rook.def(Lang::Pt), "a torre preta em a8");
        let pawn = PieceRef::new(Role::Pawn).vars("p", Lang::Pt);
        assert_eq!(fill("{p_ns}", &pawn), "peões");
    }

    #[test]
    fn french_agreement() {
        let bishop = PieceRef::on(Role::Bishop, Square::C4);
        let v = bishop.vars("p", Lang::Fr);
        assert_eq!(fill("{P} est attaqué{p_o} ; on pense {p_al}, la case {p_del}", &v), "Le fou en c4 est attaqué ; on pense au fou en c4, la case du fou en c4");
        let queen = PieceRef::new(Role::Queen);
        let v = queen.vars("p", Lang::Fr);
        assert_eq!(fill("{P} est attaqué{p_o}, prends-{p_lo} ; {p_un}", &v), "La dame est attaquée, prends-la ; une dame");
        assert_eq!(PieceRef::on(Role::Rook, Square::A8).colored(Color::Black).def(Lang::Fr), "la tour noire en a8");
        assert_eq!(PieceRef::on(Role::Knight, Square::D5).colored(Color::White).def(Lang::Fr), "le cavalier blanc en d5");
    }

    #[test]
    fn german_cases() {
        let bishop = PieceRef::on(Role::Bishop, Square::C4);
        let v = bishop.vars("p", Lang::De);
        assert_eq!(fill("{P} greift {p_acc} an", &v), "Der Läufer auf c4 greift den Läufer auf c4 an");
        assert_eq!(fill("mit {p_al}; die Rolle {p_del}", &v), "mit dem Läufer auf c4; die Rolle des Läufers auf c4");
        let pawn = PieceRef::new(Role::Pawn);
        let v = pawn.vars("p", Lang::De);
        assert_eq!(fill("{p_acc}, {p_al}, {p_del}, {p_un_acc}, {p_ns}", &v), "den Bauern, dem Bauern, des Bauern, einen Bauern, Bauern");
        let queen = PieceRef::new(Role::Queen).colored(Color::Black);
        assert_eq!(fill("{p_acc} / {p_al} / {p_lo}", &queen.vars("p", Lang::De)), "die schwarze Dame / der schwarzen Dame / sie");
        let knight = PieceRef::on(Role::Knight, Square::D5).colored(Color::White);
        assert_eq!(knight.de_def(DeCase::Acc), "den weißen Springer auf d5");
        assert_eq!(knight.def(Lang::De), "der weiße Springer auf d5");
        assert_eq!(fill("{p_ns}", &PieceRef::new(Role::Rook).vars("p", Lang::De)), "Türme");
    }

    #[test]
    fn every_key_in_every_language() {
        let keys = ["p", "P", "p_al", "p_del", "p_acc", "p_un", "p_un_acc", "p_n", "p_ns", "p_o", "p_lo", "p_er"];
        for lang in Lang::ALL {
            for role in [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen, Role::King] {
                let v = PieceRef::on(role, Square::E4).colored(Color::White).vars("p", lang);
                for key in keys {
                    assert!(v.iter().any(|(k, _)| k == key), "{lang} {role:?} missing {key}");
                }
                assert!(!fill("{p}", &v).contains('{'));
                assert!(!piece_name(role, lang).is_empty() && !piece_plural(role, lang).is_empty());
            }
            assert!(!side_name(Color::White, lang).is_empty() && !side_name(Color::Black, lang).is_empty());
        }
    }

    #[test]
    fn plurals() {
        assert_eq!(plural(0, "partie", "parties", Lang::Fr), "partie");
        assert_eq!(plural(0, "Partie", "Partien", Lang::De), "Partien");
        assert_eq!(plural(1, "jogo", "jogos", Lang::Pt), "jogo");
        assert_eq!(plural(2, "jogo", "jogos", Lang::Pt), "jogos");
    }

    #[test]
    fn lists_and_numbers() {
        let two = ["a".to_string(), "b".to_string()];
        assert_eq!(join_list(&two, Lang::De), "a und b");
        assert_eq!(join_list(&two, Lang::Fr), "a et b");
        assert_eq!(join_list(&two, Lang::Pt), "a e b");
        assert_eq!(decimal1(92.34, Lang::De), "92,3");
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(join_list(&items, Lang::Es), "a, b y c");
        assert_eq!(join_list(&items[..1], Lang::En), "a");
        assert_eq!(decimal1(92.34, Lang::Es), "92,3");
    }
}
