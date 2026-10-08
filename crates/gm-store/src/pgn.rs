//! PGN export and import.
//!
//! * [`to_pgn`] renders a standards-compliant PGN (Seven Tag Roster first, `SetUp`/`FEN` for
//!   non-standard starts, SAN with move numbers, result terminator, lines wrapped at 80 columns).
//! * [`parse_pgn`] reads one or many games: tag pairs, `{...}` / `;` comments, `%` escape lines,
//!   NAGs (`$1`) and suffix annotations (`!?`), nested variations (skipped), move numbers
//!   (`12.` / `12...`), results, `0-0` style castling, promotions (`e8=Q`, `e8Q`, `e8=q`).
//!   Moves are validated with shakmaty and returned as standard UCI (`e1g1` for castling).
//!
//! Everything is iterative (no recursion on nested variations) and bounded, so hostile input
//! cannot blow the stack or allocate without limit.

use serde::{Deserialize, Serialize};
use shakmaty::fen::Fen;
use shakmaty::san::{San, SanPlus};
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Color, EnPassantMode, Position};

/// Standard starting position.
pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Maximum plies accepted per game (longest possible legal game is ~5900 plies; real ones < 600).
pub const MAX_PLIES: usize = 2000;
/// Maximum games returned by one parse.
pub const MAX_GAMES: usize = 5000;
/// Maximum tag pairs kept per game.
const MAX_TAGS: usize = 64;
/// Maximum length of a single tag name / value.
const MAX_TAG_LEN: usize = 1024;

const ROSTER: [&str; 7] = ["Event", "Site", "Date", "Round", "White", "Black", "Result"];

/// A game parsed from PGN text.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ParsedGame {
    /// Tag pairs in file order, e.g. ("White", "Magnus").
    pub headers: Vec<(String, String)>,
    /// Full 6-field FEN of the starting position (standard start if no FEN tag).
    pub start_fen: String,
    /// UCI
    pub moves: Vec<String>,
    /// "1-0" | "0-1" | "1/2-1/2" | "*"
    pub result: String,
}

impl ParsedGame {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Normalize a result string to one of the four PGN results.
pub fn normalize_result(r: &str) -> &'static str {
    match r.trim() {
        "1-0" => "1-0",
        "0-1" => "0-1",
        "1/2-1/2" | "½-½" | "1/2" | "=" => "1/2-1/2",
        _ => "*",
    }
}

/// Parse a FEN into a standard-chess position (empty / "start" = initial position).
pub fn parse_position(fen: &str) -> Result<Chess, String> {
    let fen = fen.trim();
    if fen.is_empty() || fen == "start" || fen == "startpos" {
        return Ok(Chess::default());
    }
    if fen.len() > 128 {
        return Err("FEN too long".into());
    }
    let parsed = Fen::from_ascii(fen.as_bytes()).map_err(|e| format!("invalid FEN: {e}"))?;
    parsed
        .into_position::<Chess>(CastlingMode::Standard)
        .map_err(|e| format!("illegal position: {e}"))
}

/// Full 6-field FEN of a position.
pub fn position_fen(pos: &Chess) -> String {
    Fen::from_position(pos.clone(), EnPassantMode::Legal).to_string()
}

/// Canonical FEN for a start position string (empty / "start" -> standard FEN).
pub fn canonical_fen(fen: &str) -> Result<String, String> {
    parse_position(fen).map(|p| position_fen(&p))
}

/// Is this (canonical) FEN the standard initial position?
pub fn is_standard_start(fen: &str) -> bool {
    let t = fen.trim();
    t.is_empty() || t == "start" || t == "startpos" || t == START_FEN
}

/// Parse and legality-check one UCI move.
pub fn uci_to_move(pos: &Chess, uci: &str) -> Result<shakmaty::Move, String> {
    let uci = uci.trim();
    if uci.len() < 4 || uci.len() > 5 {
        return Err(format!("invalid UCI move {uci:?}"));
    }
    let parsed =
        UciMove::from_ascii(uci.as_bytes()).map_err(|_| format!("invalid UCI move {uci:?}"))?;
    parsed
        .to_move(pos)
        .map_err(|_| format!("illegal move {uci}"))
}

/// Standard UCI notation for a move (castling as king-to-destination).
pub fn move_to_uci(m: &shakmaty::Move) -> String {
    UciMove::from_move(m, CastlingMode::Standard).to_string()
}

/// Replay UCI moves from `start_fen`; returns the SAN of every move.
/// Errors name the 1-based ply that failed.
pub fn uci_to_sans(start_fen: &str, moves: &[String]) -> Result<Vec<String>, String> {
    let mut pos = parse_position(start_fen)?;
    let mut out = Vec::with_capacity(moves.len());
    for (i, u) in moves.iter().enumerate() {
        let m = uci_to_move(&pos, u).map_err(|e| format!("ply {}: {e}", i + 1))?;
        out.push(SanPlus::from_move_and_play_unchecked(&mut pos, &m).to_string());
    }
    Ok(out)
}

fn escape_tag(v: &str) -> String {
    let mut s = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            '\n' | '\r' | '\t' => s.push(' '),
            c if c.is_control() => {}
            c => s.push(c),
        }
    }
    s
}

fn valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || c == '_'
                || c == '+'
                || c == '#'
                || c == '='
                || c == ':'
                || c == '-'
        })
}

/// Render a PGN from tag pairs, a start FEN and UCI moves.
///
/// The Seven Tag Roster is always emitted first (missing tags get the PGN "unknown" defaults),
/// `Result` is taken from `result`, `SetUp`/`FEN` are emitted when the start position is not
/// the standard one, then remaining tags in the order given. Rendering stops at the first
/// illegal/unparseable move (the PGN stays valid). Movetext is wrapped at 80 columns.
pub fn to_pgn(
    headers: &[(String, String)],
    start_fen: &str,
    moves: &[String],
    result: &str,
) -> String {
    let result = normalize_result(result);
    let get = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };

    let (start_pos, fen_tag) = match parse_position(start_fen) {
        Ok(p) => {
            let f = position_fen(&p);
            let nonstd = f != START_FEN;
            (p, nonstd.then_some(f))
        }
        Err(_) => (Chess::default(), None),
    };

    let mut out = String::with_capacity(256 + moves.len() * 6);
    for name in ROSTER {
        let val = if name == "Result" {
            result.to_string()
        } else {
            match get(name) {
                Some(v) if !v.trim().is_empty() => v.to_string(),
                _ => match name {
                    "Date" => "????.??.??".to_string(),
                    _ => "?".to_string(),
                },
            }
        };
        out.push_str(&format!("[{name} \"{}\"]\n", escape_tag(&val)));
    }
    if let Some(f) = &fen_tag {
        out.push_str("[SetUp \"1\"]\n");
        out.push_str(&format!("[FEN \"{}\"]\n", escape_tag(f)));
    }
    for (k, v) in headers {
        if ROSTER.iter().any(|r| r.eq_ignore_ascii_case(k))
            || k.eq_ignore_ascii_case("SetUp")
            || k.eq_ignore_ascii_case("FEN")
            || !valid_tag_name(k)
        {
            continue;
        }
        out.push_str(&format!("[{k} \"{}\"]\n", escape_tag(v)));
    }
    out.push('\n');

    // Movetext tokens.
    let mut tokens: Vec<String> = Vec::with_capacity(moves.len() * 2 + 1);
    let mut pos = start_pos;
    let mut first = true;
    for u in moves.iter().take(MAX_PLIES) {
        let m = match uci_to_move(&pos, u) {
            Ok(m) => m,
            Err(_) => break,
        };
        let number = pos.fullmoves().get();
        match pos.turn() {
            Color::White => tokens.push(format!("{number}.")),
            Color::Black if first => tokens.push(format!("{number}...")),
            Color::Black => {}
        }
        first = false;
        tokens.push(SanPlus::from_move_and_play_unchecked(&mut pos, &m).to_string());
    }
    tokens.push(result.to_string());

    // Wrap at 80 columns; a move number stays glued to its move ("12. Nf3").
    let mut line = String::with_capacity(80);
    let mut i = 0;
    while i < tokens.len() {
        let mut word = tokens[i].clone();
        if word.ends_with('.') && i + 1 < tokens.len() {
            word.push(' ');
            word.push_str(&tokens[i + 1]);
            i += 1;
        }
        i += 1;
        if !line.is_empty() && line.len() + 1 + word.len() > 80 {
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    out.push_str(&line);
    out.push('\n');
    out
}

// ---------------------------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Tok {
    Tag(String, String),
    Word(String),
    OpenVar,
    CloseVar,
}

/// Tokenizer: produces tag pairs and movetext words; drops comments, NAGs and escape lines.
struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
    at_line_start: bool,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str) -> Self {
        let s = text.as_bytes();
        let i = if s.starts_with(&[0xEF, 0xBB, 0xBF]) {
            3
        } else {
            0
        };
        Lexer {
            s,
            i,
            at_line_start: true,
        }
    }

    fn text(&self, a: usize, b: usize) -> String {
        String::from_utf8_lossy(&self.s[a..b]).into_owned()
    }

    fn skip_line(&mut self) {
        while self.i < self.s.len() && self.s[self.i] != b'\n' {
            self.i += 1;
        }
    }

    fn next(&mut self) -> Option<Tok> {
        loop {
            let c = *self.s.get(self.i)?;
            match c {
                b'\n' => {
                    self.i += 1;
                    self.at_line_start = true;
                }
                b' ' | b'\t' | b'\r' | 0x0b | 0x0c => self.i += 1,
                b'%' if self.at_line_start => self.skip_line(),
                b';' => self.skip_line(),
                b'{' => {
                    self.at_line_start = false;
                    match self.s[self.i..].iter().position(|&b| b == b'}') {
                        Some(off) => self.i += off + 1,
                        None => self.i = self.s.len(),
                    }
                }
                b'(' => {
                    self.at_line_start = false;
                    self.i += 1;
                    return Some(Tok::OpenVar);
                }
                b')' => {
                    self.at_line_start = false;
                    self.i += 1;
                    return Some(Tok::CloseVar);
                }
                b'[' => {
                    self.at_line_start = false;
                    if let Some(t) = self.tag() {
                        return Some(t);
                    }
                }
                b'$' => {
                    self.at_line_start = false;
                    self.i += 1;
                    while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                        self.i += 1;
                    }
                }
                _ => {
                    self.at_line_start = false;
                    let a = self.i;
                    while self.i < self.s.len() {
                        let b = self.s[self.i];
                        if b.is_ascii_whitespace()
                            || matches!(b, b'{' | b'}' | b'(' | b')' | b'[' | b']' | b';' | b'$')
                        {
                            break;
                        }
                        self.i += 1;
                    }
                    if self.i == a {
                        // Stray delimiter such as ']' or '}': skip it.
                        self.i += 1;
                        continue;
                    }
                    return Some(Tok::Word(self.text(a, self.i)));
                }
            }
        }
    }

    /// Parses `[Name "Value"]` starting at '['. Returns None (and skips to end of line) on
    /// malformed tags.
    fn tag(&mut self) -> Option<Tok> {
        let start = self.i;
        self.i += 1;
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t') {
            self.i += 1;
        }
        let a = self.i;
        while self.i < self.s.len()
            && !self.s[self.i].is_ascii_whitespace()
            && !matches!(self.s[self.i], b'"' | b']')
        {
            self.i += 1;
        }
        let name = self.text(a, self.i);
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t') {
            self.i += 1;
        }
        if name.is_empty() || self.s.get(self.i) != Some(&b'"') {
            self.i = start;
            self.skip_line();
            return None;
        }
        self.i += 1;
        let mut val: Vec<u8> = Vec::new();
        while self.i < self.s.len() {
            let b = self.s[self.i];
            match b {
                b'\\' if matches!(self.s.get(self.i + 1), Some(b'"') | Some(b'\\')) => {
                    val.push(self.s[self.i + 1]);
                    self.i += 2;
                }
                b'"' => {
                    self.i += 1;
                    break;
                }
                b'\n' => break,
                _ => {
                    val.push(b);
                    self.i += 1;
                }
            }
        }
        // Skip to closing bracket on the same line.
        while self.i < self.s.len() && self.s[self.i] != b']' && self.s[self.i] != b'\n' {
            self.i += 1;
        }
        if self.s.get(self.i) == Some(&b']') {
            self.i += 1;
        }
        let mut name = name;
        name.truncate(MAX_TAG_LEN);
        let mut value = String::from_utf8_lossy(&val).into_owned();
        if value.len() > MAX_TAG_LEN {
            let mut cut = MAX_TAG_LEN;
            while !value.is_char_boundary(cut) {
                cut -= 1;
            }
            value.truncate(cut);
        }
        Some(Tok::Tag(name, value))
    }
}

fn result_token(w: &str) -> Option<&'static str> {
    match w {
        "1-0" => Some("1-0"),
        "0-1" => Some("0-1"),
        "1/2-1/2" | "½-½" => Some("1/2-1/2"),
        "*" => Some("*"),
        _ => None,
    }
}

/// Strip move numbers ("12." / "12..." / "12…"), annotation suffixes and normalize castling
/// and promotion spelling. Returns None if nothing move-like remains.
fn clean_move_word(w: &str) -> Option<String> {
    let mut s = w;
    // Leading move number, possibly glued to the move ("1.e4", "12...Nf6").
    let digits = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 {
        let rest = &s[digits..];
        let rest_trim = rest.trim_start_matches(['.', '…']);
        if rest_trim.len() != rest.len() {
            s = rest_trim;
        } else if rest.is_empty() {
            return None; // bare number
        }
    }
    let s = s.trim_start_matches(['.', '…']);
    // Annotation glyphs.
    let s = s.trim_end_matches(['!', '?']);
    let s = s.strip_suffix("e.p.").unwrap_or(s);
    if s.is_empty() {
        return None;
    }
    let mut m = s.replace('0', "O");
    // "0-0" -> "O-O" may wrongly touch nothing else: '0' never appears in SAN otherwise.
    if m.starts_with("o-o") {
        m = m.to_ascii_uppercase();
    }
    // Promotion: "e8Q" -> "e8=Q", "e8=q" -> "e8=Q".
    let b = m.as_bytes();
    let core_len = m.trim_end_matches(['+', '#']).len();
    if core_len >= 3 {
        let last = b[core_len - 1];
        let prev = b[core_len - 2];
        let starts_pawn = b[0].is_ascii_lowercase();
        if starts_pawn && matches!(last.to_ascii_uppercase(), b'Q' | b'R' | b'B' | b'N') {
            if prev == b'=' {
                let mut v = m.into_bytes();
                v[core_len - 1] = last.to_ascii_uppercase();
                m = String::from_utf8(v).unwrap_or_default();
            } else if prev == b'1' || prev == b'8' {
                let mut v = m.into_bytes();
                v[core_len - 1] = last.to_ascii_uppercase();
                v.insert(core_len - 1, b'=');
                m = String::from_utf8(v).unwrap_or_default();
            }
        }
    }
    if m.is_empty() {
        None
    } else {
        Some(m)
    }
}

/// Per-game parse state.
#[derive(Default)]
struct GameBuilder {
    headers: Vec<(String, String)>,
    pos: Option<Chess>,
    start_fen: String,
    moves: Vec<String>,
    result: Option<String>,
    error: Option<String>,
    has_content: bool,
    var_depth: usize,
}

impl GameBuilder {
    fn ensure_position(&mut self) {
        if self.pos.is_some() || self.error.is_some() {
            return;
        }
        let fen = self
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("FEN"))
            .map(|(_, v)| v.clone());
        let variant = self
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("Variant"))
            .map(|(_, v)| v.to_ascii_lowercase());
        if let Some(v) = variant {
            if !(v.is_empty() || v == "standard" || v == "chess" || v == "from position") {
                self.error = Some(format!("unsupported variant {v:?}"));
                return;
            }
        }
        match parse_position(fen.as_deref().unwrap_or("")) {
            Ok(p) => {
                self.start_fen = position_fen(&p);
                self.pos = Some(p);
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn push_san(&mut self, word: &str) {
        self.has_content = true;
        self.ensure_position();
        if self.error.is_some() {
            return;
        }
        let Some(pos) = self.pos.as_mut() else { return };
        if self.moves.len() >= MAX_PLIES {
            self.error = Some(format!("game longer than {MAX_PLIES} plies"));
            return;
        }
        let ply = self.moves.len() + 1;
        let san = match SanPlus::from_ascii(word.as_bytes()) {
            Ok(s) => s.san,
            Err(_) => {
                self.error = Some(format!("ply {ply}: cannot read move {word:?}"));
                return;
            }
        };
        if matches!(san, San::Null) {
            self.error = Some(format!("ply {ply}: null moves are not supported"));
            return;
        }
        match san.to_move(pos) {
            Ok(m) => {
                self.moves.push(move_to_uci(&m));
                pos.play_unchecked(&m);
            }
            Err(_) => {
                let n = pos.fullmoves().get();
                let dots = if pos.turn() == Color::White {
                    "."
                } else {
                    "..."
                };
                self.error = Some(format!("illegal move {n}{dots}{word}"));
            }
        }
    }

    fn finish(mut self) -> Option<Result<ParsedGame, String>> {
        if !self.has_content && self.headers.is_empty() {
            return None;
        }
        self.ensure_position();
        if let Some(e) = self.error {
            let who = match (
                self.headers.iter().find(|(k, _)| k == "White"),
                self.headers.iter().find(|(k, _)| k == "Black"),
            ) {
                (Some((_, w)), Some((_, b))) => format!(" ({w} vs {b})"),
                _ => String::new(),
            };
            return Some(Err(format!("{e}{who}")));
        }
        let result = self
            .result
            .or_else(|| {
                self.headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("Result"))
                    .map(|(_, v)| normalize_result(v).to_string())
            })
            .unwrap_or_else(|| "*".to_string());
        Some(Ok(ParsedGame {
            headers: self.headers,
            start_fen: self.start_fen,
            moves: self.moves,
            result,
        }))
    }
}

/// Parse games, returning successfully parsed games plus per-game error messages.
pub fn parse_pgn_lenient(text: &str) -> (Vec<ParsedGame>, Vec<String>) {
    let mut games = Vec::new();
    let mut errors = Vec::new();
    let mut lex = Lexer::new(text);
    let mut cur = GameBuilder::default();
    let mut in_movetext = false;

    let flush =
        |b: GameBuilder, games: &mut Vec<ParsedGame>, errors: &mut Vec<String>| match b.finish() {
            Some(Ok(g)) => games.push(g),
            Some(Err(e)) if errors.len() < 100 => errors.push(e),
            Some(Err(_)) => {}
            None => {}
        };

    while let Some(tok) = lex.next() {
        if games.len() >= MAX_GAMES {
            break;
        }
        match tok {
            Tok::Tag(k, v) => {
                if in_movetext {
                    // A tag after movetext without a result terminator starts a new game.
                    flush(std::mem::take(&mut cur), &mut games, &mut errors);
                    in_movetext = false;
                }
                if cur.headers.len() < MAX_TAGS {
                    cur.headers.push((k, v));
                }
            }
            Tok::OpenVar => {
                in_movetext = true;
                cur.var_depth += 1;
            }
            Tok::CloseVar => {
                cur.var_depth = cur.var_depth.saturating_sub(1);
            }
            Tok::Word(w) => {
                if let Some(r) = result_token(&w) {
                    if cur.var_depth > 0 {
                        continue;
                    }
                    cur.result = Some(r.to_string());
                    cur.has_content = true;
                    flush(std::mem::take(&mut cur), &mut games, &mut errors);
                    in_movetext = false;
                    continue;
                }
                in_movetext = true;
                if cur.var_depth > 0 {
                    continue;
                }
                if let Some(m) = clean_move_word(&w) {
                    cur.push_san(&m);
                }
            }
        }
    }
    flush(cur, &mut games, &mut errors);
    (games, errors)
}

/// Parse one or more games from PGN text.
///
/// Games that contain illegal moves are skipped when at least one other game parsed;
/// if no game could be parsed, the first error is returned.
pub fn parse_pgn(text: &str) -> Result<Vec<ParsedGame>, String> {
    let (games, errors) = parse_pgn_lenient(text);
    if games.is_empty() {
        return Err(errors
            .into_iter()
            .next()
            .unwrap_or_else(|| "no games found in PGN".to_string()));
    }
    for e in &errors {
        tracing::warn!("skipping PGN game: {e}");
    }
    Ok(games)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const OPERA: &str = r#"[Event "A Night at the Opera"]
[Site "Paris FRA"]
[Date "1858.??.??"]
[Round "?"]
[White "Paul Morphy"]
[Black "Duke Karl / Count Isouard"]
[Result "1-0"]
[ECO "C41"]

1.e4 e5 2.Nf3 d6 3.d4 Bg4 {This is a weak move already.--Fischer} 4.dxe5 Bxf3
5.Qxf3 dxe5 6.Bc4 Nf6 7.Qb3 Qe7 8.Nc3 c6 9.Bg5 {Black is in what's like a
zugzwang position here. He can't develop the [Queen's] knight because the pawn
is hanging, the bishop is blocked because of the Queen.--Fischer} b5 10.Nxb5!
cxb5 11.Bxb5+ Nbd7 12.O-O-O Rd8 13.Rxd7 Rxd7 14.Rd1 Qe6 15.Bxd7+ Nxd7 16.Qb8+!
Nxb8 17.Rd8# 1-0
"#;

    #[test]
    fn opera_game() {
        let games = parse_pgn(OPERA).unwrap();
        assert_eq!(games.len(), 1);
        let g = &games[0];
        assert_eq!(g.result, "1-0");
        assert_eq!(g.moves.len(), 33);
        assert_eq!(g.moves[0], "e2e4");
        assert_eq!(g.moves[22], "e1c1"); // 12.O-O-O
        assert_eq!(g.moves.last().unwrap(), "d1d8");
        assert_eq!(g.header("white"), Some("Paul Morphy"));
        assert_eq!(g.start_fen, START_FEN);
    }

    #[test]
    fn roundtrip() {
        let g = &parse_pgn(OPERA).unwrap()[0];
        let out = to_pgn(&g.headers, &g.start_fen, &g.moves, &g.result);
        assert!(out.starts_with("[Event \"A Night at the Opera\"]\n[Site \"Paris FRA\"]"));
        assert!(out.contains("[ECO \"C41\"]"));
        assert!(out.contains("12. O-O-O Rd8"));
        assert!(out.trim_end().ends_with("17. Rd8# 1-0"));
        for line in out.lines() {
            assert!(line.len() <= 80, "line too long: {line}");
        }
        let back = &parse_pgn(&out).unwrap()[0];
        assert_eq!(back.moves, g.moves);
        assert_eq!(back.result, g.result);
    }

    #[test]
    fn variations_nags_castling_zero_promotion() {
        let pgn = r#"[White "A"][Black "B"]
1. e4 $1 (1. d4 d5 (1... Nf6 2. c4) 2. c4) 1... e5!? 2. Nf3 Nc6 3. Bc4 Bc5 4. 0-0 Nf6
; line comment 1-0
% escape line
5. d3 *

[White "C"]
[Black "D"]
[SetUp "1"]
[FEN "8/P7/8/8/8/8/8/k6K w - - 0 1"]

1. a8Q+ Kb2 2. Qb8+ 1/2-1/2
"#;
        let games = parse_pgn(pgn).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(
            games[0].moves,
            vec!["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "f8c5", "e1g1", "g8f6", "d2d3"]
        );
        assert_eq!(games[0].result, "*");
        assert_eq!(games[1].moves, vec!["a7a8q", "a1b2", "a8b8"]);
        assert_eq!(games[1].result, "1/2-1/2");
        let out = to_pgn(
            &games[1].headers,
            &games[1].start_fen,
            &games[1].moves,
            "1/2-1/2",
        );
        assert!(out.contains("[SetUp \"1\"]"));
        assert!(out.contains("1. a8=Q+ Kb2 2. Qb8+ 1/2-1/2"));
    }

    #[test]
    fn black_to_move_start() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let out = to_pgn(&[], fen, &["e7e5".into(), "g1f3".into()], "*");
        assert!(out.contains("1... e5 2. Nf3 *"), "{out}");
        let back = &parse_pgn(&out).unwrap()[0];
        assert_eq!(back.moves, vec!["e7e5", "g1f3"]);
        assert_eq!(back.start_fen, fen);
    }

    #[test]
    fn bad_input_is_error_not_panic() {
        assert!(parse_pgn("").is_err());
        assert!(parse_pgn("1. e4 e5 2. Ke3 *").is_err());
        let _ = parse_pgn("[[[[ {{{{ ((((( $$$ 1. ... ]]]] }}}} )))");
        let _ = parse_pgn("[White \"unterminated\n1. e4 e5 *");
        let _ = parse_pgn("1. e4 e5 2. Nf3 {never closed");
        let nested = "(".repeat(100_000) + "1. e4";
        let _ = parse_pgn(&nested);
        // One bad game among good ones is skipped.
        let two = "1. e4 e5 1-0\n\n1. e5 *\n\n1. d4 d5 0-1";
        let g = parse_pgn(two).unwrap();
        assert_eq!(g.len(), 2);
        assert!(parse_pgn("[Variant \"Atomic\"]\n1. e4 *").is_err());
    }

    #[test]
    fn tag_escapes() {
        let h = vec![("White".to_string(), "Mr \"Quote\" \\ Back".to_string())];
        let out = to_pgn(&h, "", &[], "*");
        let g = &parse_pgn(&out).unwrap()[0];
        assert_eq!(g.header("White"), Some("Mr \"Quote\" \\ Back"));
    }
}
