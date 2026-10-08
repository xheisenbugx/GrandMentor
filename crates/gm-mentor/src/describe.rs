//! Static position description: 3-6 short, beginner-friendly ideas about a position, in
//! every [`Lang`]. Spanish sides are plural subjects ("las blancas tienen..."), and piece
//! phrases agree in gender ("el caballo negro en d5", "la torre blanca en a1").

use shakmaty::{Bitboard, Board, CastlingSide, Chess, Color, File, Position, Rank, Role, Square};

use gm_content::words::{capitalize, es_piece, fill, join_list, kv, piece_name, piece_plural, side_name, PieceRef};
use gm_content::Lang;

use crate::phrase::Picker;
use crate::tactics::{self, piece_material};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Opening,
    Middlegame,
    Endgame,
}

pub(crate) fn phase(pos: &Chess) -> Phase {
    let b = pos.board();
    let npm = piece_material(b, Color::White) + piece_material(b, Color::Black);
    let minors_home = undeveloped_minors(b, Color::White).len() + undeveloped_minors(b, Color::Black).len();
    if npm <= 2600 || (b.queens().is_empty() && npm <= 3300) {
        Phase::Endgame
    } else if (pos.fullmoves().get() <= 10 && minors_home >= 3) || pos.fullmoves().get() <= 6 {
        Phase::Opening
    } else {
        Phase::Middlegame
    }
}

fn home_minors(c: Color) -> [(Square, Role); 4] {
    match c {
        Color::White => [(Square::B1, Role::Knight), (Square::G1, Role::Knight), (Square::C1, Role::Bishop), (Square::F1, Role::Bishop)],
        Color::Black => [(Square::B8, Role::Knight), (Square::G8, Role::Knight), (Square::C8, Role::Bishop), (Square::F8, Role::Bishop)],
    }
}

pub(crate) fn undeveloped_minors(b: &Board, c: Color) -> Vec<(Square, Role)> {
    home_minors(c)
        .into_iter()
        .filter(|(sq, r)| b.piece_at(*sq).map(|p| p.color == c && p.role == *r).unwrap_or(false))
        .collect()
}

fn file_bb(f: File) -> Bitboard {
    Bitboard::from_file(f)
}

fn adjacent_files(f: File) -> Bitboard {
    let mut bb = Bitboard::EMPTY;
    if let Some(l) = f.offset(-1) {
        bb |= file_bb(l);
    }
    if let Some(r) = f.offset(1) {
        bb |= file_bb(r);
    }
    bb
}

/// Squares strictly in front of `sq` from `c`'s perspective, on files f-1..f+1.
fn front_span(c: Color, sq: Square) -> Bitboard {
    let files = file_bb(sq.file()) | adjacent_files(sq.file());
    let mut ranks = Bitboard::EMPTY;
    let r = sq.rank() as i32;
    for i in 0..8 {
        let ahead = match c {
            Color::White => i > r,
            Color::Black => i < r,
        };
        if ahead {
            ranks |= Bitboard::from_rank(Rank::new(i as u32));
        }
    }
    files & ranks
}

pub(crate) fn passed_pawns(b: &Board, c: Color) -> Vec<Square> {
    let theirs = b.by_color(!c) & b.pawns();
    let mut out = Vec::new();
    for sq in b.by_color(c) & b.pawns() {
        if (front_span(c, sq) & theirs).is_empty() {
            out.push(sq);
        }
    }
    // Most advanced first.
    out.sort_by_key(|s| match c {
        Color::White => -(s.rank() as i32),
        Color::Black => s.rank() as i32,
    });
    out
}

fn isolated_pawns(b: &Board, c: Color) -> Vec<Square> {
    let ours = b.by_color(c) & b.pawns();
    ours.into_iter().filter(|sq| (adjacent_files(sq.file()) & ours).is_empty()).collect()
}

fn doubled_files(b: &Board, c: Color) -> Vec<File> {
    let ours = b.by_color(c) & b.pawns();
    File::ALL.into_iter().filter(|f| (file_bb(*f) & ours).count() >= 2).collect()
}

/// Pick the template for `lang`.
fn t<'a>(lang: Lang, en: &'a str, es: &'a str) -> &'a str {
    match lang {
        Lang::En => en,
        Lang::Es => es,
    }
}

fn tl<'a>(lang: Lang, en: &'a [&'a str], es: &'a [&'a str]) -> &'a [&'a str] {
    match lang {
        Lang::En => en,
        Lang::Es => es,
    }
}

/// Side as a sentence subject, capitalized ("White" / "Las blancas").
fn side(c: Color, lang: Lang) -> String {
    capitalize(side_name(c, lang))
}

fn side_label(c: Color, to_move: Color, lang: Lang) -> String {
    if c == to_move {
        fill(t(lang, "{s} (to move)", "{s} (con el turno)"), &kv(&[("s", &side(c, lang))]))
    } else {
        side(c, lang)
    }
}

fn material_sentence(b: &Board, lang: Lang) -> String {
    let w = tactics::material_points(b, Color::White);
    let bl = tactics::material_points(b, Color::Black);
    let diff = w - bl;
    if diff == 0 {
        // Note piece imbalances even when points are equal.
        let count = |c: Color, r: Role| (b.by_color(c) & b.by_role(r)).count() as i32;
        let wb = count(Color::White, Role::Bishop);
        let bb = count(Color::Black, Role::Bishop);
        let pair = |c: Color| {
            fill(
                t(
                    lang,
                    "Material is equal, but {s} has the bishop pair — a long-term plus in open positions.",
                    "El material está igualado, pero {s} tienen la pareja de alfiles: una ventaja a largo plazo en posiciones abiertas.",
                ),
                &kv(&[("s", side_name(c, lang))]),
            )
        };
        if wb == 2 && bb < 2 {
            return pair(Color::White);
        }
        if bb == 2 && wb < 2 {
            return pair(Color::Black);
        }
        return t(lang, "Material is level.", "El material está igualado.").into();
    }
    let (leader, n) = if diff > 0 { (Color::White, diff) } else { (Color::Black, -diff) };
    let what = describe_material_edge(b, leader, lang);
    let advice = if n >= 3 {
        t(lang, "trade pieces (not pawns) and head for a simple endgame", "cambiar piezas (no peones) e ir a un final sencillo")
    } else {
        t(lang, "keep it safe and look to trade down", "conservar la ventaja y buscar cambios")
    };
    let tpl = match (lang, n == 1) {
        (Lang::En, true) => "{S} is up {n} point of material{what} — the plan is to {advice}.",
        (Lang::En, false) => "{S} is up {n} points of material{what} — the plan is to {advice}.",
        (Lang::Es, true) => "{S} tienen {n} punto de ventaja material{what}: el plan es {advice}.",
        (Lang::Es, false) => "{S} tienen {n} puntos de ventaja material{what}: el plan es {advice}.",
    };
    fill(tpl, &kv(&[("S", &side(leader, lang)), ("n", &n.to_string()), ("what", &what), ("advice", advice)]))
}

fn describe_material_edge(b: &Board, leader: Color, lang: Lang) -> String {
    let mut extra = Vec::new();
    let mut all_single = true;
    for r in [Role::Queen, Role::Rook, Role::Bishop, Role::Knight, Role::Pawn] {
        let a = (b.by_color(leader) & b.by_role(r)).count() as i32;
        let o = (b.by_color(!leader) & b.by_role(r)).count() as i32;
        if a > o {
            let n = a - o;
            if n == 1 {
                extra.push(match lang {
                    Lang::En => piece_name(r, lang).to_string(),
                    Lang::Es => format!("{} {}", es_piece(r).un(), es_piece(r).sing),
                });
            } else {
                all_single = false;
                extra.push(format!("{n} {}", piece_plural(r, lang)));
            }
        }
    }
    if extra.is_empty() || extra.len() > 3 {
        String::new()
    } else if all_single {
        match lang {
            Lang::En => format!(" (an extra {})", extra.join(" and ")),
            Lang::Es => format!(" ({} de más)", join_list(&extra, lang)),
        }
    } else {
        match lang {
            Lang::En => format!(" (extra: {})", extra.join(", ")),
            Lang::Es => format!(" (de más: {})", extra.join(", ")),
        }
    }
}

fn king_exposed(pos: &Chess, c: Color) -> bool {
    let b = pos.board();
    let Some(k) = b.king_of(c) else { return false };
    let center_files = k.file() >= File::C && k.file() <= File::F;
    let home = match c {
        Color::White => k.rank() <= Rank::Second,
        Color::Black => k.rank() >= Rank::Seventh,
    };
    let can_castle = pos.castles().has(c, CastlingSide::KingSide) || pos.castles().has(c, CastlingSide::QueenSide);
    let enemy_queen = !(b.by_color(!c) & b.queens()).is_empty();
    // Open file in front of the king.
    let own_pawns = b.by_color(c) & b.pawns();
    let open_in_front = (file_bb(k.file()) & own_pawns).is_empty();
    enemy_queen && ((!home) || (center_files && !can_castle && pos.fullmoves().get() >= 10) || (open_in_front && center_files && pos.fullmoves().get() >= 8))
}

fn pawn_shield_weak(b: &Board, c: Color) -> bool {
    let Some(k) = b.king_of(c) else { return false };
    if k.file() > File::C && k.file() < File::G {
        return false;
    }
    let shield_rank = match c {
        Color::White => k.rank().offset(1),
        Color::Black => k.rank().offset(-1),
    };
    let Some(sr) = shield_rank else { return false };
    let files = file_bb(k.file()) | adjacent_files(k.file());
    let near = files & (Bitboard::from_rank(sr) | sr.offset(if c == Color::White { 1 } else { -1 }).map(Bitboard::from_rank).unwrap_or(Bitboard::EMPTY));
    (near & b.by_color(c) & b.pawns()).count() <= 1 && !(b.by_color(!c) & b.queens()).is_empty()
}

/// One idea about the position. `threat` marks immediate dangers / tactics (used by the chat
/// to answer "any threats?").
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Idea {
    pub text: String,
    pub threat: bool,
}

/// Rule-based list of ideas (3-6) about a position.
pub fn describe_position(fen: &str, lang: Lang) -> Vec<String> {
    let Ok(pos) = gm_engine::parse_fen(fen) else {
        return vec![t(lang, "That position doesn't look valid — try setting it up again.", "Esa posición no parece válida: intenta colocarla de nuevo.").to_string()];
    };
    describe(&pos, lang).into_iter().map(|i| i.text).collect()
}

pub(crate) fn describe(pos: &Chess, lang: Lang) -> Vec<Idea> {
    let b = pos.board();
    let stm = pos.turn();
    let p = Picker::new(&[&gm_engine::to_fen(pos)]);
    let mut ideas: Vec<Idea> = Vec::new();
    let plain = |s: String| Idea { text: s, threat: false };
    let threat = |s: String| Idea { text: s, threat: true };
    let us = side(stm, lang);
    let them = side(!stm, lang);

    // Game over states.
    if pos.is_checkmate() {
        return [
            fill(t(lang, "Checkmate — {S} has won the game.", "Jaque mate: {s} han ganado la partida."), &kv(&[("S", &them), ("s", side_name(!stm, lang))])),
            t(lang, "Step back through the moves to see how the attack came together.", "Repasa las jugadas para ver cómo se construyó el ataque.").to_string(),
            t(lang, "Notice which pieces covered the king's escape squares.", "Fíjate en qué piezas cubrían las casillas de escape del rey.").to_string(),
        ]
        .into_iter()
        .map(plain)
        .collect();
    }
    if pos.is_stalemate() {
        return [
            t(
                lang,
                "Stalemate — the game is a draw because the side to move has no legal moves but isn't in check.",
                "Ahogado: la partida es tablas porque el bando que mueve no tiene jugadas legales y no está en jaque.",
            ),
            t(lang, "When you're winning, always make sure the opponent has a legal move left!", "Cuando vayas ganando, ¡asegúrate siempre de que tu rival tenga alguna jugada legal!"),
            t(lang, "Material doesn't matter anymore once it's stalemate.", "Cuando hay ahogado, el material ya no importa."),
        ]
        .into_iter()
        .map(|s| plain(s.to_string()))
        .collect();
    }
    if pos.is_insufficient_material() {
        return [
            t(lang, "Neither side has enough material to checkmate — it's a draw.", "Ningún bando tiene material suficiente para dar mate: son tablas."),
            t(lang, "Remember: a lone king plus a bishop or knight can't force mate.", "Recuerda: un rey con solo un alfil o un caballo no puede forzar el mate."),
            t(lang, "Try practising basic mates in the Endgames section.", "Practica los mates básicos en la sección de Finales."),
        ]
        .into_iter()
        .map(|s| plain(s.to_string()))
        .collect();
    }

    // 1. Immediate tactics.
    if pos.is_check() {
        ideas.push(threat(fill(
            t(
                lang,
                "{S} is in check and must deal with it first: move the king, block, or capture the checker.",
                "{S} están en jaque y deben resolverlo primero: mover el rey, tapar el jaque o capturar la pieza que lo da.",
            ),
            &kv(&[("S", &us)]),
        )));
    }
    if let Some(m) = tactics::mate_in_one(pos) {
        ideas.push(threat(fill(
            t(lang, "{S} has checkmate in one: {m}!", "¡{S} tienen mate en una: {m}!"),
            &kv(&[("S", &us), ("m", &tactics::san(pos, &m))]),
        )));
    } else if let Some((swapped, m)) = tactics::null_move_mate_threat(pos) {
        let mv = tactics::san(&swapped, &m);
        let tpl = match (tactics::is_back_rank_mate(&swapped, &m), lang) {
            (true, Lang::En) => "Watch out: {T} threatens {m}, checkmate on the back rank — {s} must defend.",
            (false, Lang::En) => "Watch out: {T} threatens {m}, checkmate — {s} must defend.",
            (true, Lang::Es) => "Cuidado: {t} amenazan {m}, mate del pasillo; {s} deben defenderse.",
            (false, Lang::Es) => "Cuidado: {t} amenazan {m}, jaque mate; {s} deben defenderse.",
        };
        ideas.push(threat(fill(tpl, &kv(&[("T", &them), ("t", side_name(!stm, lang)), ("m", &mv), ("s", &match lang {
            Lang::En => us.clone(),
            Lang::Es => side_name(stm, lang).to_string(),
        })]))));
    }

    // 2. Hanging pieces (for the side to move these are opportunities; for the other, threats).
    let opp_hanging = tactics::hanging_pieces(b, !stm);
    if let Some(h) = opp_hanging.first() {
        let cap = tactics::best_capture_on(pos, h.square).map(|m| tactics::san(pos, &m));
        let mut v = PieceRef::on(h.role, h.square).colored(h.color).vars("p", lang);
        v.extend(kv(&[("s", side_name(stm, lang)), ("S", &us)]));
        let tpl = match (&cap, h.undefended) {
            (Some(_), true) => t(lang, "{P} is undefended — {S} can grab it with {c}.", "{P} está indefens{p_o}: {s} pueden capturar{p_lo} con {c}."),
            (Some(_), false) => t(lang, "{P} is under-protected — {S} wins material with {c}.", "{P} está mal defendid{p_o}: {s} ganan material con {c}."),
            (None, _) => t(lang, "{P} is loose — look for ways to attack it.", "{P} está suelt{p_o}: busca maneras de atacar{p_lo}."),
        };
        if let Some(c) = &cap {
            v.extend(kv(&[("c", c)]));
        }
        ideas.push(threat(fill(tpl, &v)));
    }
    let own_hanging = tactics::hanging_pieces(b, stm);
    if let Some(h) = own_hanging.first() {
        let v = PieceRef::on(h.role, h.square).colored(stm).vars("p", lang);
        ideas.push(threat(fill(
            t(
                lang,
                "{P} is under attack — move it, defend it, or create a bigger threat.",
                "{P} está atacad{p_o}: muéve{p_lo}, defiénde{p_lo} o crea una amenaza mayor.",
            ),
            &v,
        )));
    }

    // 3. Material.
    ideas.push(plain(material_sentence(b, lang)));

    let ph = phase(pos);

    // 4. Development & king safety.
    if ph == Phase::Opening || pos.fullmoves().get() <= 14 {
        for c in [stm, !stm] {
            let undeveloped = undeveloped_minors(b, c);
            if undeveloped.len() >= 2 && pos.fullmoves().get() >= 4 {
                let mut uniq: Vec<Role> = undeveloped.iter().map(|(_, r)| *r).collect();
                uniq.dedup();
                let names: Vec<String> = uniq.iter().map(|r| piece_plural(*r, lang).to_string()).collect();
                ideas.push(plain(fill(
                    t(
                        lang,
                        "{S} still has {n} minor pieces at home ({names}) — developing them is a priority.",
                        "{S} todavía tienen {n} piezas menores sin desarrollar ({names}): sacarlas es la prioridad.",
                    ),
                    &kv(&[("S", &side_label(c, stm, lang)), ("n", &undeveloped.len().to_string()), ("names", &join_list(&names, lang))]),
                )));
                break;
            }
        }
    }
    for c in [stm, !stm] {
        let castled_rights = pos.castles().has(c, CastlingSide::KingSide) || pos.castles().has(c, CastlingSide::QueenSide);
        let king = PieceRef::new(Role::King).colored(c).def(lang);
        if king_exposed(pos, c) {
            ideas.push(threat(fill(
                t(
                    lang,
                    "{K} looks exposed — attacking it (or tucking it away) is the key theme.",
                    "{K} parece expuesto: atacarlo (o ponerlo a salvo) es el tema clave.",
                ),
                &kv(&[("K", &capitalize(&king))]),
            )));
            break;
        } else if castled_rights && pos.fullmoves().get() >= 6 && ph != Phase::Endgame {
            ideas.push(plain(fill(
                t(
                    lang,
                    "{S} hasn't castled yet — castling soon keeps the king safe and connects the rooks.",
                    "{S} aún no han enrocado: enrocar pronto pone el rey a salvo y conecta las torres.",
                ),
                &kv(&[("S", &side_label(c, stm, lang))]),
            )));
            break;
        } else if pawn_shield_weak(b, c) && ph == Phase::Middlegame {
            ideas.push(threat(fill(
                t(
                    lang,
                    "The pawn cover around {S}'s king is thin — keep an eye on attacks there.",
                    "La protección de peones de {k} es escasa: vigila los ataques por ese lado.",
                ),
                &kv(&[("S", &side(c, lang)), ("k", &king)]),
            )));
            break;
        }
    }

    // 5. Passed pawns.
    for c in [stm, !stm] {
        if let Some(sq) = passed_pawns(b, c).first() {
            let advanced = match c {
                Color::White => sq.rank() >= Rank::Fifth,
                Color::Black => sq.rank() <= Rank::Fourth,
            };
            if advanced || ph == Phase::Endgame {
                let tail = p.pick(
                    61,
                    tl(
                        lang,
                        &["push it with support, and the opponent must block it with a piece.", "passed pawns must be pushed!", "it can become a queen if nobody stops it."],
                        &["avánzalo con apoyo, y el rival tendrá que bloquearlo con una pieza.", "¡los peones pasados hay que avanzarlos!", "puede convertirse en dama si nadie lo detiene."],
                    ),
                );
                ideas.push(plain(fill(
                    t(lang, "{S} has a passed pawn on {sq} — {tail}", "{S} tienen un peón pasado en {sq}: {tail}"),
                    &kv(&[("S", &side(c, lang)), ("sq", &sq.to_string()), ("tail", tail)]),
                )));
                break;
            }
        }
    }

    // 6. Endgame advice.
    if ph == Phase::Endgame {
        ideas.push(plain(
            p.pick(
                62,
                tl(
                    lang,
                    &["It's an endgame: bring the king toward the center — it's a strong piece now.", "Endgame time: activate your king and create a passed pawn."],
                    &["Estamos en un final: lleva el rey hacia el centro, ahora es una pieza fuerte.", "Hora del final: activa tu rey y crea un peón pasado."],
                ),
            )
            .to_string(),
        ));
    }

    // 7. Open files for rooks.
    let all_pawns = b.pawns();
    let open: Vec<File> = File::ALL.into_iter().filter(|f| (file_bb(*f) & all_pawns).is_empty()).collect();
    if !open.is_empty() && ph != Phase::Opening {
        let rooks_on: Vec<Color> = [Color::White, Color::Black]
            .into_iter()
            .filter(|c| open.iter().any(|f| !(file_bb(*f) & b.by_color(*c) & b.rooks_and_queens()).is_empty()))
            .collect();
        let letters: Vec<String> = open.iter().take(2).map(|f| f.char().to_string()).collect();
        if rooks_on.len() == 1 {
            ideas.push(plain(fill(
                t(
                    lang,
                    "{S} controls the open {f}-file with a heavy piece — try to invade on the 7th rank.",
                    "{S} controlan la columna abierta {f} con una pieza pesada: intenta invadir por la séptima fila.",
                ),
                &kv(&[("S", &side(rooks_on[0], lang)), ("f", &letters[0])]),
            )));
        } else if rooks_on.is_empty() && !b.rooks().is_empty() {
            let text = match (lang, letters.len() > 1) {
                (Lang::En, true) => format!("The {}-file and {}-file are open — whoever puts a rook there first gains an edge.", letters[0], letters[1]),
                (Lang::En, false) => format!("The {}-file is open — whoever puts a rook there first gains an edge.", letters[0]),
                (Lang::Es, true) => format!("Las columnas {} y {} están abiertas: quien ponga primero una torre ahí gana ventaja.", letters[0], letters[1]),
                (Lang::Es, false) => format!("La columna {} está abierta: quien ponga primero una torre ahí gana ventaja.", letters[0]),
            };
            ideas.push(plain(text));
        }
    }

    // 8. Pawn weaknesses.
    for c in [!stm, stm] {
        let iso = isolated_pawns(b, c);
        let dbl = doubled_files(b, c);
        if let Some(sq) = iso.first().filter(|_| ph != Phase::Opening) {
            let pawn = PieceRef::on(Role::Pawn, *sq).colored(c).def(lang);
            ideas.push(plain(fill(
                t(lang, "{P} is isolated — no pawn can defend it, so it's a target.", "{P} está aislado: ningún peón puede defenderlo, así que es un objetivo."),
                &kv(&[("P", &capitalize(&pawn))]),
            )));
            break;
        }
        if let Some(f) = dbl.first().filter(|_| ph != Phase::Opening) {
            ideas.push(plain(fill(
                t(lang, "{S} has doubled pawns on the {f}-file, a small long-term weakness.", "{S} tienen peones doblados en la columna {f}, una pequeña debilidad a largo plazo."),
                &kv(&[("S", &side(c, lang)), ("f", &f.char().to_string())]),
            )));
            break;
        }
    }

    // 9. Center.
    if ph == Phase::Opening {
        let center = [Square::D4, Square::E4, Square::D5, Square::E5];
        let count = |c: Color| center.iter().filter(|s| b.piece_at(**s).map(|pc| pc.color == c && pc.role == Role::Pawn).unwrap_or(false)).count();
        let (w, bl) = (count(Color::White), count(Color::Black));
        let text = if w > bl {
            t(
                lang,
                "White has more pawns in the center — Black should challenge it with pawn breaks like ...c5 or ...d5.",
                "Las blancas tienen más peones en el centro: las negras deberían desafiarlo con rupturas como ...c5 o ...d5.",
            )
        } else if bl > w {
            t(lang, "Black has more central pawns — White should fight back for the center.", "Las negras tienen más peones centrales: las blancas deberían luchar por el centro.")
        } else {
            p.pick(
                63,
                tl(
                    lang,
                    &[
                        "Opening principles: control the center, develop knights and bishops, and castle early.",
                        "Follow the basics: develop a new piece each move and don't move the same piece twice without a reason.",
                    ],
                    &[
                        "Principios de apertura: controla el centro, desarrolla caballos y alfiles y enroca pronto.",
                        "Lo básico: desarrolla una pieza nueva en cada jugada y no muevas la misma pieza dos veces sin motivo.",
                    ],
                ),
            )
        };
        ideas.push(plain(text.to_string()));
    }

    // Keep 3..=6, de-duplicated.
    let mut seen = std::collections::HashSet::new();
    ideas.retain(|i| seen.insert(i.text.clone()));
    let fillers = tl(
        lang,
        &[
            "Before every move, check for checks, captures and threats — for both sides.",
            "Find your least active piece and look for a better square for it.",
            "Ask yourself: what does the opponent want to do next?",
        ],
        &[
            "Antes de cada jugada, busca jaques, capturas y amenazas, para ambos bandos.",
            "Encuentra tu pieza menos activa y busca una casilla mejor para ella.",
            "Pregúntate: ¿qué quiere hacer el rival a continuación?",
        ],
    );
    let mut i = 0;
    while ideas.len() < 3 && i < fillers.len() {
        ideas.push(Idea { text: fillers[i].to_string(), threat: false });
        i += 1;
    }
    ideas.truncate(6);
    ideas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_position_has_ideas() {
        let ideas = describe_position("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", Lang::En);
        assert!((3..=6).contains(&ideas.len()), "{ideas:?}");
        assert!(ideas.iter().any(|i| i.contains("Material is level")));
    }

    #[test]
    fn hanging_and_material() {
        let ideas = describe_position("4k3/8/8/3n4/8/8/8/3RK3 w - - 0 1", Lang::En);
        assert!(ideas.iter().any(|i| i.contains("knight on d5")), "{ideas:?}");
        assert!(ideas.iter().any(|i| i.contains("up")), "{ideas:?}");
    }

    #[test]
    fn mate_threat_noticed() {
        let ideas = describe_position("6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1", Lang::En);
        assert!(ideas.iter().any(|i| i.contains("Ra8#")), "{ideas:?}");
    }

    #[test]
    fn invalid_fen() {
        assert_eq!(describe_position("garbage", Lang::En).len(), 1);
        assert!(describe_position("garbage", Lang::Es)[0].starts_with("Esa posición"));
    }

    #[test]
    fn spanish_ideas() {
        let ideas = describe_position("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", Lang::Es);
        assert!(ideas.iter().any(|i| i == "El material está igualado."), "{ideas:?}");
        // Black knight hanging on d5: feminine/masculine agreement + plural side subject.
        let ideas = describe_position("4k3/8/8/3n4/8/8/8/3RK3 w - - 0 1", Lang::Es);
        assert!(ideas.iter().any(|i| i.starts_with("El caballo negro en d5 está indefenso: las blancas pueden capturarlo con Rxd5.")), "{ideas:?}");
        assert!(ideas.iter().any(|i| i.starts_with("Las blancas tienen 2 puntos de ventaja material")), "{ideas:?}");
        let ideas = describe_position("6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1", Lang::Es);
        assert!(ideas.iter().any(|i| i.contains("¡Las blancas tienen mate en una: Ra8#!")), "{ideas:?}");
        // A white rook hanging: feminine.
        let ideas = describe_position("4k3/8/8/8/8/8/1r6/1R2K3 w - - 0 1", Lang::Es);
        assert!(ideas.iter().any(|i| i.contains("torre negra en b2")), "{ideas:?}");
        let ideas = describe_position("4k3/8/8/8/3b4/8/8/R3K3 b - - 0 1", Lang::Es);
        assert!(ideas.iter().any(|i| i.contains("La torre blanca en a1 está indefensa")), "{ideas:?}");
        for i in &ideas {
            assert!(!i.contains(" the ") && !i.contains("White") && !i.contains("Black"), "{i}");
        }
    }
}
