//! Rule-based chat answers (used when no LLM key is configured or the LLM call fails).
//!
//! Questions are routed by keywords in every supported language (so "¿qué debo hacer?" and
//! "what should I do?" both ask for the best move); the answer is written in the requested
//! [`Lang`].

use gm_content::words::{capitalize, fill, kv, side_name as side_word, PieceRef};
use gm_content::Lang;
use shakmaty::{Chess, Color, Position};

use crate::describe::{self, Phase};
use crate::explain::plain_move_comment;
use crate::phrase::Picker;
use crate::tactics;
use crate::ChatRequest;

/// One parsed engine line like "+0.45: Nf3 Nc6 Bb5".
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EngineLine {
    /// White-POV evaluation in pawns (mates mapped to +-1000 + distance info).
    pub pawns: Option<f32>,
    /// Mate in N (positive = White mates).
    pub mate: Option<i32>,
    pub score_text: String,
    pub moves: Vec<String>,
}

pub(crate) fn parse_engine_line(s: &str) -> Option<EngineLine> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (score_text, rest) = match s.split_once(':') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => ("", s),
    };
    let moves: Vec<String> = rest
        .split_whitespace()
        .filter(|t| !t.ends_with('.') && !t.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .map(|t| t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.').to_string())
        .filter(|t| !t.is_empty())
        .take(24)
        .collect();
    let st = score_text.replace(' ', "");
    let (mut pawns, mut mate) = (None, None);
    let lower = st.to_ascii_lowercase();
    if let Some(idx) = lower.find(['m', '#']) {
        let sign = if lower.starts_with('-') { -1 } else { 1 };
        let digits: String = lower[idx + 1..].chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<i32>() {
            mate = Some(sign * n.max(1));
        }
    } else if let Ok(v) = st.trim_start_matches('+').parse::<f32>() {
        if v.is_finite() {
            pawns = Some(v.clamp(-99.0, 99.0));
        }
    }
    Some(EngineLine { pawns, mate, score_text: score_text.to_string(), moves })
}

/// Pick the string for `lang` from `[en, es, pt, fr, de]`.
fn t(lang: Lang, s: [&'static str; 5]) -> &'static str {
    match lang {
        Lang::En => s[0],
        Lang::Es => s[1],
        Lang::Pt => s[2],
        Lang::Fr => s[3],
        Lang::De => s[4],
    }
}

fn tl<'a>(lang: Lang, en: &'a [&'a str], es: &'a [&'a str], pt: &'a [&'a str], fr: &'a [&'a str], de: &'a [&'a str]) -> &'a [&'a str] {
    match lang {
        Lang::En => en,
        Lang::Es => es,
        Lang::Pt => pt,
        Lang::Fr => fr,
        Lang::De => de,
    }
}

/// Plain-language verdict for a White-POV engine line.
pub(crate) fn eval_words(line: &EngineLine, lang: Lang) -> Option<String> {
    if let Some(m) = line.mate {
        let side = capitalize(side_word(if m > 0 { Color::White } else { Color::Black }, lang));
        let tpl = t(
            lang,
            [
                "{S} has a forced checkmate in {n}.",
                "{S} tienen un mate forzado en {n}.",
                "{S} têm um mate forçado em {n}.",
                "{S} ont un mat forcé en {n}.",
                "{S} hat ein erzwungenes Matt in {n}.",
            ],
        );
        return Some(fill(tpl, &kv(&[("S", &side), ("n", &m.abs().to_string())])));
    }
    let v = line.pawns?;
    let (side, a) = if v >= 0.0 { (Color::White, v) } else { (Color::Black, -v) };
    let tpl = if a < 0.3 {
        t(
            lang,
            [
                "The position is roughly equal ({e}).",
                "La posición está más o menos igualada ({e}).",
                "A posição está mais ou menos igualada ({e}).",
                "La position est à peu près égale ({e}).",
                "Die Stellung ist ungefähr ausgeglichen ({e}).",
            ],
        )
    } else if a < 1.0 {
        t(
            lang,
            [
                "{S} is slightly better ({e}).",
                "{S} están algo mejor ({e}).",
                "{S} estão um pouco melhor ({e}).",
                "{S} sont légèrement mieux ({e}).",
                "{S} steht etwas besser ({e}).",
            ],
        )
    } else if a < 2.5 {
        t(
            lang,
            [
                "{S} is clearly better ({e}).",
                "{S} están claramente mejor ({e}).",
                "{S} estão claramente melhor ({e}).",
                "{S} sont nettement mieux ({e}).",
                "{S} steht klar besser ({e}).",
            ],
        )
    } else {
        t(
            lang,
            [
                "{S} is winning ({e}).",
                "{S} están ganando ({e}).",
                "{S} estão ganhando ({e}).",
                "{S} ont une position gagnante ({e}).",
                "{S} steht auf Gewinn ({e}).",
            ],
        )
    };
    Some(fill(tpl, &kv(&[("S", &capitalize(side_word(side, lang))), ("e", &line.score_text)])))
}

/// Try to reconstruct the position before the last move by replaying `moves_san` from the
/// standard start. Only works for games that began from the initial position.
fn previous_position(req: &ChatRequest, current: &Chess) -> Option<(Chess, shakmaty::Move)> {
    if req.moves_san.is_empty() || req.moves_san.len() > 600 {
        return None;
    }
    let mut pos = Chess::default();
    let mut prev = None;
    for s in &req.moves_san {
        let m = tactics::parse_san(&pos, s)?;
        prev = Some((pos.clone(), m.clone()));
        pos.play_unchecked(&m);
    }
    if pos.board() == current.board() && pos.turn() == current.turn() {
        prev
    } else {
        None
    }
}

/// Maps a localized SAN piece letter to the English one (es: R D T A C; pt: R D T B C;
/// fr: R D T F C; de: K D T L S). `None` for English or letters that aren't localized.
fn local_piece_letter(c: char, lang: Lang) -> Option<char> {
    match (lang, c) {
        (Lang::En, _) => None,
        (Lang::Es, 'C') | (Lang::Pt, 'C') | (Lang::Fr, 'C') | (Lang::De, 'S') => Some('N'),
        (Lang::Es, 'A') | (Lang::Pt, 'B') | (Lang::Fr, 'F') | (Lang::De, 'L') => Some('B'),
        (Lang::Es | Lang::Pt | Lang::Fr | Lang::De, 'D') => Some('Q'),
        (Lang::Es | Lang::Pt | Lang::Fr | Lang::De, 'T') => Some('R'),
        (Lang::Es | Lang::Pt | Lang::Fr, 'R') | (Lang::De, 'K') => Some('K'),
        _ => None,
    }
}

/// SAN written with localized piece letters ("Cf3", "Fb5", "Sf3") to English SAN.
fn localized_san(tok: &str, lang: Lang) -> Option<String> {
    let mut chars = tok.chars();
    let first = local_piece_letter(chars.next()?, lang)?;
    Some(std::iter::once(first).chain(chars).collect())
}

fn find_move_in_question(pos: &Chess, q: &str, lang: Lang) -> Option<shakmaty::Move> {
    for raw in q.split(|c: char| c.is_whitespace() || matches!(c, ',' | '?' | '¿' | '¡' | '"' | '\'' | '’' | '(' | ')' | ';' | '«' | '»' | '„' | '“' | '”')) {
        let tok = raw.trim_matches(|c: char| c == '.' || c == '!' || c == ':');
        let tok = tok.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        if tok.len() < 2 || tok.len() > 8 {
            continue;
        }
        // Avoid treating plain words as moves: SAN starts with a piece letter, a file, or O.
        let first = tok.chars().next().unwrap_or(' ');
        let local_letter = local_piece_letter(first, lang).is_some();
        if !(matches!(first, 'K' | 'Q' | 'R' | 'B' | 'N' | 'O' | '0') || ('a'..='h').contains(&first) || local_letter) {
            continue;
        }
        if tok.chars().all(|c| c.is_ascii_alphabetic()) && !tok.starts_with('O') && tok.len() > 3 {
            continue; // a plain word like "bishop" / "dama"
        }
        // English SAN / UCI first, then the localized piece letters.
        if let Some(m) = tactics::parse_any_move(pos, tok) {
            return Some(m);
        }
        if let Some(m) = localized_san(tok, lang).and_then(|s| tactics::parse_san(pos, &s)) {
            return Some(m);
        }
    }
    None
}

/// Lowercase, strip accents and inverted punctuation, so keyword routing works for
/// "¿Qué debo hacer?" as well as "que debo hacer", "Été" as "ete" and "Läufer" as "laufer".
fn normalize(q: &str) -> String {
    q.to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '¿' | '¡'))
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            '’' | '`' => '\'',
            c => c,
        })
        .collect()
}

fn has_any(q: &str, words: &[&str]) -> bool {
    words.iter().any(|w| q.contains(w))
}

/// Whole-word match (for short words like "oi" or "hi" that occur inside other words).
fn has_word(q: &str, words: &[&str]) -> bool {
    q.split(|c: char| !c.is_alphanumeric()).any(|w| words.contains(&w))
}

/// Keyword lists; each intent lists English, Spanish, Portuguese, French and German
/// (normalized: lowercase, accent-free) cues.
const LAST_MOVE: &[&str] = &[
    "last move", "my move", "that move", "previous move", "why was", "why is that", "what was wrong", "mistake", "blunder",
    // es
    "ultima jugada", "mi jugada", "esa jugada", "jugada anterior", "por que fue", "por que es malo", "por que es mala",
    "que estuvo mal", "que hice mal", "error", "fallo", "me equivoque",
    // pt
    "ultimo lance", "meu lance", "esse lance", "este lance", "lance anterior", "por que foi", "por que e ruim", "por que esse lance",
    "o que deu errado", "o que fiz de errado", "errei", "erro", "capivarada",
    // fr
    "dernier coup", "mon coup", "ce coup", "coup precedent", "pourquoi c'etait", "pourquoi ce coup", "pourquoi est-ce mauvais",
    "qu'est-ce qui n'allait pas", "je me suis trompe", "erreur", "gaffe",
    // de
    "letzter zug", "letzten zug", "mein zug", "meinen zug", "dieser zug", "diesen zug", "vorheriger zug", "warum war",
    "warum ist dieser", "was war falsch", "was habe ich falsch", "fehler", "patzer",
];
const BEST_MOVE: &[&str] = &[
    "best move", "what should", "what do i", "what to play", "next move", "suggest", "hint", "which move", "what move", "help me",
    // es
    "mejor jugada", "mejor movimiento", "que debo hacer", "que deberia", "que hago", "que juego", "que jugar", "que muevo",
    "siguiente jugada", "sugier", "sugerencia", "pista", "ayuda", "que jugada", "cual jugada", "que movimiento",
    // pt
    "melhor lance", "melhor jogada", "o que devo fazer", "o que eu faco", "o que faco", "o que jogar", "que lance", "qual lance",
    "proximo lance", "sugest", "dica", "ajuda", "me ajud",
    // fr
    "meilleur coup", "que dois-je faire", "que dois-je jouer", "que faire", "quoi jouer", "que jouer", "je joue quoi", "quel coup",
    "prochain coup", "suggere", "conseil", "indice", "aide-moi", "aide moi",
    // de
    "bester zug", "besten zug", "was soll ich", "was spiele ich", "was ziehe ich", "welcher zug", "welchen zug", "nachster zug",
    "nachsten zug", "vorschlag", "tipp", "hinweis", "hilfe", "hilf mir",
];
const EVAL: &[&str] = &[
    "winning", "who is better", "who's better", "evaluation", "eval", "advantage", "score", "am i",
    // es
    "ganando", "quien va mejor", "quien esta mejor", "quien gana", "evaluacion", "ventaja", "como voy", "voy bien", "voy mal",
    // pt
    "ganhando", "quem esta melhor", "quem esta ganhando", "quem ganha", "avaliacao", "vantagem", "como estou",
    // fr
    "qui gagne", "qui est mieux", "qui est meilleur", "evaluation", "avantage", "ou j'en suis", "ou en suis-je", "je suis mieux",
    // de
    "wer gewinnt", "wer steht besser", "wer ist besser", "bewertung", "vorteil", "wie stehe ich", "stehe ich",
];
const THREATS: &[&str] = &[
    "threat", "danger", "attack", "hanging", "safe",
    // es
    "amenaza", "peligro", "ataque", "atacad", "colgad", "a salvo", "segur",
    // pt
    "ameaca", "perigo", "pendurad",
    // fr
    "menace", "attaqu", "en prise", "securite",
    // de
    "drohung", "droht", "gefahr", "angriff", "angegriffen", "hangt", "ungedeckt", "sicher",
];
const OPENING: &[&str] = &["opening", "apertura", "abertura", "ouverture", "eroffnung"];
const ENDGAME: &[&str] = &["endgame", "final", "endspiel"];
const PLAN: &[&str] = &[
    "plan", "idea", "strategy", "what now", "explain", "position", "what's going on", "what is going on", "understand",
    // es
    "estrategia", "y ahora", "ahora que", "explica", "posicion", "que pasa", "que esta pasando", "entender", "entiendo",
    // pt
    "plano", "e agora", "explique", "posicao", "o que esta acontecendo",
    // fr
    "strategie", "et maintenant", "que se passe", "comprendre", "comprends",
    // de
    "idee", "was nun", "und jetzt", "erklar", "stellung", "was passiert", "verstehe",
];
const GREETING: &[&str] = &[
    "hello", "hi ", "hey", "thanks", "thank you", "hola", "gracias", "buenas", "buenos dias", "obrigad", "bom dia", "boa tarde",
    "boa noite", "bonjour", "bonsoir", "salut", "merci", "hallo", "danke", "guten tag", "guten morgen",
];
/// Short greetings matched as whole words only.
const GREETING_WORDS: &[&str] = &["hi", "oi", "ola", "moin", "servus", "coucou"];

fn bullets(items: &[String]) -> String {
    items.iter().map(|i| format!("• {i}")).collect::<Vec<_>>().join("\n")
}

fn best_move_text(pos: &Chess, lines: &[EngineLine], p: &Picker, lang: Lang) -> Option<String> {
    let line = lines.first()?;
    let first = line.moves.first()?;
    let m = tactics::parse_any_move(pos, first)?;
    let san = tactics::san(pos, &m);
    let comment = plain_move_comment(pos, &m, lang);
    let cont: Vec<&str> = line.moves.iter().skip(1).take(4).map(String::as_str).collect();
    let lead = p.pick(
        71,
        tl(
            lang,
            &["The engine's top choice is", "I'd play", "The strongest move here is"],
            &["La primera opción del motor es", "Yo jugaría", "La jugada más fuerte aquí es"],
            &["A primeira opção do motor é", "Eu jogaria", "O lance mais forte aqui é"],
            &["Le premier choix du moteur est", "Je jouerais", "Le coup le plus fort ici est"],
            &["Die erste Wahl der Engine ist", "Mein Tipp:", "Der stärkste Zug hier ist"],
        ),
    );
    let mut s = format!("{lead} **{san}**");
    if !line.score_text.is_empty() {
        s.push_str(&format!(" ({})", line.score_text));
    }
    s.push('.');
    if !comment.quiet {
        s.push(' ');
        s.push_str(&comment.text);
    }
    if !cont.is_empty() {
        let tpl = t(
            lang,
            [
                " A likely continuation: {c}.",
                " Una continuación probable: {c}.",
                " Uma continuação provável: {c}.",
                " Une suite probable : {c}.",
                " Eine wahrscheinliche Fortsetzung: {c}.",
            ],
        );
        s.push_str(&fill(tpl, &kv(&[("c", &cont.join(" "))])));
    }
    if let Some(second) = lines.get(1).and_then(|l| l.moves.first()) {
        if let Some(m2) = tactics::parse_any_move(pos, second) {
            let tpl = t(
                lang,
                [
                    " **{m}** is a decent alternative.",
                    " **{m}** es una alternativa decente.",
                    " **{m}** é uma alternativa razoável.",
                    " **{m}** est une alternative correcte.",
                    " **{m}** ist eine ordentliche Alternative.",
                ],
            );
            s.push_str(&fill(tpl, &kv(&[("m", &tactics::san(pos, &m2))])));
        }
    }
    Some(s)
}

fn hint_without_engine(pos: &Chess, lang: Lang) -> String {
    if let Some(m) = tactics::mate_in_one(pos) {
        let tpl = t(
            lang,
            [
                "There's a checkmate in one: **{m}**!",
                "¡Hay un mate en una: **{m}**!",
                "Há um mate em um: **{m}**!",
                "Il y a un mat en un coup : **{m}** !",
                "Es gibt ein Matt in einem Zug: **{m}**!",
            ],
        );
        return fill(tpl, &kv(&[("m", &tactics::san(pos, &m))]));
    }
    let opp = tactics::hanging_pieces(pos.board(), !pos.turn());
    if let Some(h) = opp.first() {
        if let Some(c) = tactics::best_capture_on(pos, h.square) {
            let mut v = PieceRef::on(h.role, h.square).vars("p", lang);
            v.extend(kv(&[("c", &tactics::san(pos, &c))]));
            let tpl = t(
                lang,
                [
                    "Look at {p} — **{c}** wins material.",
                    "Fíjate en {p}: **{c}** gana material.",
                    "Olhe para {p}: **{c}** ganha material.",
                    "Regarde {p} : **{c}** gagne du matériel.",
                    "Schau dir {p_acc} an – **{c}** gewinnt Material.",
                ],
            );
            return fill(tpl, &v);
        }
    }
    let own = tactics::hanging_pieces(pos.board(), pos.turn());
    if let Some(h) = own.first() {
        let mut v = PieceRef::new(h.role).vars("p", lang);
        v.extend(kv(&[("sq", &h.square.to_string())]));
        let tpl = t(
            lang,
            [
                "First, take care of your {p_n} on {sq} — it's under attack.",
                "Primero, ocúpate de tu {p_n} en {sq}: está atacad{p_o}.",
                "Primeiro, proteja {p} em {sq}: {p_er} está sendo atacad{p_o}.",
                "D'abord, mets à l'abri {p} en {sq} : {p_er} est attaqué{p_o}.",
                "Kümmere dich zuerst um {p_acc} auf {sq} – {p_er} wird angegriffen.",
            ],
        );
        return fill(tpl, &v);
    }
    t(
        lang,
        [
            "Start with a quick safety check: any checks, captures or threats? If not, improve your worst-placed piece.",
            "Empieza con un repaso de seguridad: ¿hay jaques, capturas o amenazas? Si no, mejora tu pieza peor colocada.",
            "Comece com uma checagem rápida de segurança: há xeques, capturas ou ameaças? Se não, melhore sua peça mais mal posicionada.",
            "Commence par une vérification rapide : y a-t-il des échecs, des captures ou des menaces ? Sinon, améliore ta pièce la plus mal placée.",
            "Beginne mit einem kurzen Sicherheitscheck: Gibt es Schachgebote, Schlagzüge oder Drohungen? Wenn nicht, verbessere deine am schlechtesten stehende Figur.",
        ],
    )
    .to_string()
}

/// Build a full rule-based answer in `lang`.
pub(crate) fn answer(req: &ChatRequest, lang: Lang) -> String {
    let q = normalize(&req.question);
    let pos = match gm_engine::parse_fen(&req.fen) {
        Ok(p) => p,
        Err(_) => {
            return t(
                lang,
                [
                    "I couldn't read the current position, so let me give general advice: before each move, look for checks, captures and threats, then develop your pieces, control the center and keep your king safe.",
                    "No he podido leer la posición actual, así que te doy un consejo general: antes de cada jugada, busca jaques, capturas y amenazas; después desarrolla tus piezas, controla el centro y mantén tu rey a salvo.",
                    "Não consegui ler a posição atual, então aqui vai um conselho geral: antes de cada lance, procure xeques, capturas e ameaças; depois desenvolva suas peças, controle o centro e mantenha seu rei em segurança.",
                    "Je n'ai pas pu lire la position actuelle, alors voici un conseil général : avant chaque coup, cherche les échecs, les captures et les menaces ; ensuite, développe tes pièces, contrôle le centre et garde ton roi à l'abri.",
                    "Ich konnte die aktuelle Stellung nicht lesen, deshalb ein allgemeiner Tipp: Suche vor jedem Zug nach Schachgeboten, Schlagzügen und Drohungen; entwickle dann deine Figuren, kontrolliere das Zentrum und halte deinen König sicher.",
                ],
            )
            .into();
        }
    };
    let p = Picker::new(&[&req.fen, &req.question]);
    let lines: Vec<EngineLine> = req.engine_lines.iter().take(5).filter_map(|l| parse_engine_line(l)).collect();
    let ideas = describe::describe(&pos, lang);
    let texts: Vec<String> = ideas.iter().map(|i| i.text.clone()).collect();

    if pos.is_game_over() {
        return bullets(&texts);
    }

    // A specific move named in the question.
    let wants_last = has_any(&q, LAST_MOVE);
    if let Some(m) = find_move_in_question(&pos, &req.question, lang) {
        let comment = plain_move_comment(&pos, &m, lang);
        let san = tactics::san(&pos, &m);
        let mut out = comment.text;
        if let Some(top) = lines.first().and_then(|l| l.moves.first()).and_then(|t| tactics::parse_any_move(&pos, t)) {
            let tail = if top == m {
                let tpl = t(
                    lang,
                    [
                        " Good news: **{m}** is the engine's top choice!",
                        " ¡Buenas noticias: **{m}** es la primera opción del motor!",
                        " Boa notícia: **{m}** é a primeira opção do motor!",
                        " Bonne nouvelle : **{m}** est le premier choix du moteur !",
                        " Gute Nachricht: **{m}** ist die erste Wahl der Engine!",
                    ],
                );
                fill(tpl, &kv(&[("m", &san)]))
            } else {
                let top_san = tactics::san(&pos, &top);
                let tpl = if comment.bad {
                    t(
                        lang,
                        [
                            " Instead, consider **{m}**.",
                            " En su lugar, considera **{m}**.",
                            " Em vez disso, considere **{m}**.",
                            " À la place, envisage **{m}**.",
                            " Erwäge stattdessen **{m}**.",
                        ],
                    )
                } else {
                    t(
                        lang,
                        [
                            " The engine slightly prefers **{m}**.",
                            " El motor prefiere ligeramente **{m}**.",
                            " O motor prefere levemente **{m}**.",
                            " Le moteur préfère légèrement **{m}**.",
                            " Die Engine bevorzugt leicht **{m}**.",
                        ],
                    )
                };
                fill(tpl, &kv(&[("m", &top_san)]))
            };
            out.push_str(&tail);
        }
        return out;
    }
    if wants_last {
        if let Some((prev, m)) = previous_position(req, &pos) {
            let comment = plain_move_comment(&prev, &m, lang);
            let mut out = comment.text;
            if !comment.bad {
                out.push_str(t(
                    lang,
                    [
                        " I don't see an immediate tactical problem with it — if the engine disagrees, it's probably a deeper positional issue. Use the Best button to compare.",
                        " No le veo ningún problema táctico inmediato: si el motor no está de acuerdo, seguramente sea algo posicional más profundo. Usa el botón de mejor jugada para comparar.",
                        " Não vejo nenhum problema tático imediato: se o motor discorda, provavelmente é algo posicional mais profundo. Use o botão de melhor lance para comparar.",
                        " Je n'y vois aucun problème tactique immédiat : si le moteur n'est pas d'accord, c'est sans doute une question positionnelle plus profonde. Utilise le bouton du meilleur coup pour comparer.",
                        " Ich sehe kein unmittelbares taktisches Problem – wenn die Engine anderer Meinung ist, geht es wohl um etwas Positionelles. Vergleiche mit dem Button für den besten Zug.",
                    ],
                ));
            }
            if let Some(b) = best_move_text(&pos, &lines, &p, lang) {
                out.push_str(t(lang, ["\n\nNow: ", "\n\nAhora: ", "\n\nAgora: ", "\n\nMaintenant : ", "\n\nJetzt: "]));
                out.push_str(&b);
            }
            return out;
        }
    }

    if has_any(&q, BEST_MOVE) {
        return match best_move_text(&pos, &lines, &p, lang) {
            Some(b) => b,
            None => hint_without_engine(&pos, lang),
        };
    }

    if has_any(&q, EVAL) {
        let mut out = lines.first().and_then(|l| eval_words(l, lang)).unwrap_or_else(|| {
            t(
                lang,
                [
                    "I don't have an engine evaluation right now, but here's what I see:",
                    "Ahora mismo no tengo la evaluación del motor, pero esto es lo que veo:",
                    "No momento não tenho a avaliação do motor, mas é isto que eu vejo:",
                    "Je n'ai pas d'évaluation du moteur pour l'instant, mais voici ce que je vois :",
                    "Gerade habe ich keine Engine-Bewertung, aber das sehe ich:",
                ],
            )
            .to_string()
        });
        out.push('\n');
        out.push_str(&bullets(&texts.iter().take(3).cloned().collect::<Vec<_>>()));
        return out;
    }

    if has_any(&q, THREATS) {
        let threats: Vec<String> = ideas.iter().filter(|i| i.threat).map(|i| i.text.clone()).collect();
        if threats.is_empty() {
            let tpl = t(
                lang,
                [
                    "No immediate threats that I can see — {S} is free to improve the position. {hint}",
                    "No veo amenazas inmediatas: {s} pueden mejorar su posición con calma. {hint}",
                    "Não vejo ameaças imediatas: {s} podem melhorar a posição com calma. {hint}",
                    "Je ne vois pas de menace immédiate : {s} peuvent améliorer leur position tranquillement. {hint}",
                    "Ich sehe keine unmittelbaren Drohungen – {S} kann die Stellung in Ruhe verbessern. {hint}",
                ],
            );
            return fill(
                tpl,
                &kv(&[
                    ("S", &capitalize(side_word(pos.turn(), lang))),
                    ("s", side_word(pos.turn(), lang)),
                    ("hint", &hint_without_engine(&pos, lang)),
                ]),
            );
        }
        return bullets(&threats);
    }

    let ph = describe::phase(&pos);
    if has_any(&q, OPENING) && ph == Phase::Opening {
        let items: Vec<String> = tl(
            lang,
            &[
                "Control the center with pawns and pieces (e4, d4, Nf3, Nc3...).",
                "Develop knights and bishops before moving the queen out.",
                "Castle early to keep the king safe and connect the rooks.",
            ],
            &[
                "Controla el centro con peones y piezas (e4, d4, Nf3, Nc3...).",
                "Desarrolla caballos y alfiles antes de sacar la dama.",
                "Enroca pronto para poner el rey a salvo y conectar las torres.",
            ],
            &[
                "Controle o centro com peões e peças (e4, d4, Nf3, Nc3...).",
                "Desenvolva cavalos e bispos antes de tirar a dama.",
                "Faça o roque cedo para deixar o rei seguro e conectar as torres.",
            ],
            &[
                "Contrôle le centre avec des pions et des pièces (e4, d4, Nf3, Nc3...).",
                "Développe cavaliers et fous avant de sortir la dame.",
                "Roque tôt pour mettre le roi à l'abri et relier les tours.",
            ],
            &[
                "Kontrolliere das Zentrum mit Bauern und Figuren (e4, d4, Nf3, Nc3...).",
                "Entwickle Springer und Läufer, bevor du die Dame herausbringst.",
                "Rochiere früh, um den König zu sichern und die Türme zu verbinden.",
            ],
        )
        .iter()
        .map(|s| s.to_string())
        .collect();
        let header = t(
            lang,
            [
                "In the opening, three things matter most:",
                "En la apertura, lo que más importa son tres cosas:",
                "Na abertura, três coisas importam mais:",
                "Dans l'ouverture, trois choses comptent avant tout :",
                "In der Eröffnung zählen vor allem drei Dinge:",
            ],
        );
        return format!("{header}\n{}", bullets(&items));
    }
    if has_any(&q, ENDGAME) && ph == Phase::Endgame {
        let items: Vec<String> = tl(
            lang,
            &[
                "Activate the king — in the endgame it's a fighting piece.",
                "Create a passed pawn and push it with support.",
                "Rooks belong behind passed pawns (yours or the opponent's).",
            ],
            &[
                "Activa el rey: en el final es una pieza de combate.",
                "Crea un peón pasado y avánzalo con apoyo.",
                "Las torres van detrás de los peones pasados (tuyos o del rival).",
            ],
            &[
                "Ative o rei: no final ele é uma peça de combate.",
                "Crie um peão passado e avance-o com apoio.",
                "As torres ficam atrás dos peões passados (seus ou do adversário).",
            ],
            &[
                "Active le roi : en finale, c'est une pièce de combat.",
                "Crée un pion passé et pousse-le avec du soutien.",
                "Les tours se placent derrière les pions passés (les tiens ou ceux de l'adversaire).",
            ],
            &[
                "Aktiviere den König – im Endspiel ist er eine Kampffigur.",
                "Schaffe einen Freibauern und schiebe ihn mit Unterstützung vor.",
                "Türme gehören hinter Freibauern (eigene oder gegnerische).",
            ],
        )
        .iter()
        .map(|s| s.to_string())
        .collect();
        return bullets(&items);
    }

    if has_any(&q, PLAN) {
        let mut out = String::new();
        if let Some(e) = lines.first().and_then(|l| eval_words(l, lang)) {
            out.push_str(&e);
            out.push('\n');
        }
        out.push_str(&bullets(&texts));
        if let Some(b) = best_move_text(&pos, &lines, &p, lang) {
            out.push_str("\n\n");
            out.push_str(&b);
        }
        return out;
    }

    if has_any(&q, GREETING) || has_word(&q, GREETING_WORDS) {
        return p
            .pick(
                72,
                tl(
                    lang,
                    &[
                        "Hi! I'm Mentor Mira. Ask me about the best move, the plan, threats, or why a move was good or bad.",
                        "Happy to help! Try asking \"What's the plan?\" or \"Why is Nf3 good here?\"",
                    ],
                    &[
                        "¡Hola! Soy Mentor Mira. Pregúntame por la mejor jugada, el plan, las amenazas o por qué una jugada fue buena o mala.",
                        "¡Encantada de ayudarte! Prueba a preguntar «¿Cuál es el plan?» o «¿Por qué es buena Nf3 aquí?».",
                    ],
                    &[
                        "Oi! Eu sou Mentor Mira. Pergunte sobre o melhor lance, o plano, as ameaças ou por que um lance foi bom ou ruim.",
                        "Fico feliz em ajudar! Experimente perguntar \"Qual é o plano?\" ou \"Por que Nf3 é bom aqui?\"",
                    ],
                    &[
                        "Salut ! Je suis Mentor Mira. Demande-moi le meilleur coup, le plan, les menaces, ou pourquoi un coup était bon ou mauvais.",
                        "Avec plaisir ! Essaie de demander « Quel est le plan ? » ou « Pourquoi Nf3 est-il bon ici ? »",
                    ],
                    &[
                        "Hallo! Ich bin Mentor Mira. Frag mich nach dem besten Zug, dem Plan, Drohungen oder warum ein Zug gut oder schlecht war.",
                        "Ich helfe gern! Frag zum Beispiel „Was ist der Plan?“ oder „Warum ist Nf3 hier gut?“",
                    ],
                ),
            )
            .to_string();
    }

    // Default overview.
    let mut out = String::new();
    if let Some(e) = lines.first().and_then(|l| eval_words(l, lang)) {
        out.push_str(&e);
        out.push(' ');
    }
    let tpl = t(
        lang,
        [
            "It's {S}'s move. Here's what stands out:\n",
            "Juegan {s}. Esto es lo que destaca:\n",
            "Jogam {s}. Isto é o que se destaca:\n",
            "{S} jouent. Voici ce qui ressort :\n",
            "{S} ist am Zug. Das fällt auf:\n",
        ],
    );
    let big = match lang {
        Lang::En => side_word(pos.turn(), Lang::En).to_string(),
        Lang::Es | Lang::Pt | Lang::Fr | Lang::De => capitalize(side_word(pos.turn(), lang)),
    };
    out.push_str(&fill(tpl, &kv(&[("S", &big), ("s", side_word(pos.turn(), lang))])));
    out.push_str(&bullets(&texts.iter().take(4).cloned().collect::<Vec<_>>()));
    if let Some(b) = best_move_text(&pos, &lines, &p, lang) {
        out.push_str("\n\n");
        out.push_str(&b);
    }
    out
}

/// Side-to-move helper used by the LLM prompt (always English: the prompt is English).
pub(crate) fn side_name(c: Color) -> &'static str {
    tactics::color_name(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_engine_lines() {
        let l = parse_engine_line("+0.45: Nf3 Nc6 Bb5").expect("line");
        assert_eq!(l.pawns, Some(0.45));
        assert_eq!(l.moves, vec!["Nf3", "Nc6", "Bb5"]);
        let l = parse_engine_line("-M3: Qh4+ g3 Qxg3#").expect("line");
        assert_eq!(l.mate, Some(-3));
        let l = parse_engine_line("#2: 1. Qh5 Nf6 2. Qxf7#").expect("line");
        assert_eq!(l.mate, Some(2));
        assert_eq!(l.moves, vec!["Qh5", "Nf6", "Qxf7#"]);
    }

    #[test]
    fn localized_piece_letters() {
        assert_eq!(localized_san("Cf3", Lang::Es).as_deref(), Some("Nf3"));
        assert_eq!(localized_san("Aa3", Lang::Es).as_deref(), Some("Ba3"));
        assert_eq!(localized_san("Bb5", Lang::Pt).as_deref(), Some("Bb5"));
        assert_eq!(localized_san("Cc3", Lang::Pt).as_deref(), Some("Nc3"));
        assert_eq!(localized_san("Fb5", Lang::Fr).as_deref(), Some("Bb5"));
        assert_eq!(localized_san("Dh5", Lang::Fr).as_deref(), Some("Qh5"));
        assert_eq!(localized_san("Sf3", Lang::De).as_deref(), Some("Nf3"));
        assert_eq!(localized_san("Lb5", Lang::De).as_deref(), Some("Bb5"));
        assert_eq!(localized_san("Te1", Lang::De).as_deref(), Some("Re1"));
        assert_eq!(localized_san("Ke2", Lang::De).as_deref(), Some("Ke2"));
        assert_eq!(localized_san("Nf3", Lang::En), None);
        assert_eq!(localized_san("Lb5", Lang::Fr), None);
        // English SAN wins, then localized letters.
        let pos = gm_engine::parse_fen("rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2").expect("fen");
        for (q, lang) in [("warum ist Sf3 gut?", Lang::De), ("pourquoi Cf3 ?", Lang::Fr), ("por que Cf3?", Lang::Pt), ("is Nf3 good?", Lang::De)] {
            let m = find_move_in_question(&pos, q, lang).expect(q);
            assert_eq!(tactics::san(&pos, &m), "Nf3", "{q}");
        }
        assert!(find_move_in_question(&pos, "warum ist dieser Zug schlecht?", Lang::De).is_none());
        assert!(find_move_in_question(&pos, "pourquoi ce coup est mauvais ?", Lang::Fr).is_none());
    }

    #[test]
    fn normalizes_accents_in_every_language() {
        assert_eq!(normalize("Qual é a posição?"), "qual e a posicao?");
        assert_eq!(normalize("Où en suis-je ? Ça va"), "ou en suis-je ? ca va");
        assert_eq!(normalize("Erkläre die Stellung, nächster Zug"), "erklare die stellung, nachster zug");
        assert!(has_word("oi, tudo bem?", GREETING_WORDS));
        assert!(!has_word("le roi noir", GREETING_WORDS));
    }

    #[test]
    fn eval_words_in_every_language() {
        let line = parse_engine_line("+1.40: Nf3").expect("line");
        let mate = parse_engine_line("-M2: Qh4").expect("line");
        for lang in Lang::ALL {
            let a = eval_words(&line, lang).expect("words");
            let b = eval_words(&mate, lang).expect("words");
            assert!(a.contains("+1.40") && !a.contains('{'), "{lang}: {a}");
            assert!(b.contains('2') && !b.contains('{'), "{lang}: {b}");
        }
        assert_eq!(eval_words(&line, Lang::De).as_deref(), Some("Weiß steht klar besser (+1.40)."));
        assert_eq!(eval_words(&mate, Lang::Fr).as_deref(), Some("Les Noirs ont un mat forcé en 2."));
        assert_eq!(eval_words(&line, Lang::Pt).as_deref(), Some("As brancas estão claramente melhor (+1.40)."));
    }
}
