//! Static position description: 3-6 short, beginner-friendly ideas about a position, in
//! every [`Lang`]. Spanish sides are plural subjects ("las blancas tienen..."), and piece
//! phrases agree in gender ("el caballo negro en d5", "la torre blanca en a1").

use shakmaty::{Bitboard, Board, CastlingSide, Chess, Color, File, Position, Rank, Role, Square};

use gm_content::words::{capitalize, fill, join_list, kv, piece_name, piece_plural, side_name, PieceRef};
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

/// Pick the template for `lang` from `[en, es, pt, fr, de]`.
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

/// Side as a sentence subject, capitalized ("White" / "Las blancas" / "Les Blancs" / "Weiß").
fn side(c: Color, lang: Lang) -> String {
    capitalize(side_name(c, lang))
}

fn side_label(c: Color, to_move: Color, lang: Lang) -> String {
    if c == to_move {
        fill(t(lang, ["{s} (to move)", "{s} (con el turno)", "{s} (com a vez)", "{s} (au trait)", "{s} (am Zug)"]), &kv(&[("s", &side(c, lang))]))
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
                    [
                        "Material is equal, but {s} has the bishop pair — a long-term plus in open positions.",
                        "El material está igualado, pero {s} tienen la pareja de alfiles: una ventaja a largo plazo en posiciones abiertas.",
                        "O material está igualado, mas {s} têm o par de bispos: uma vantagem de longo prazo em posições abertas.",
                        "Le matériel est égal, mais {s} ont la paire de fous : un atout à long terme dans les positions ouvertes.",
                        "Das Material ist ausgeglichen, aber {s} hat das Läuferpaar – ein langfristiger Vorteil in offenen Stellungen.",
                    ],
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
        return t(lang, ["Material is level.", "El material está igualado.", "O material está igualado.", "Le matériel est égal.", "Das Material ist ausgeglichen."]).into();
    }
    let (leader, n) = if diff > 0 { (Color::White, diff) } else { (Color::Black, -diff) };
    let what = describe_material_edge(b, leader, lang);
    let advice = if n >= 3 {
        t(
            lang,
            [
                "trade pieces (not pawns) and head for a simple endgame",
                "cambiar piezas (no peones) e ir a un final sencillo",
                "trocar peças (não peões) e buscar um final simples",
                "d'échanger des pièces (pas des pions) et d'aller vers une finale simple",
                "Figuren (keine Bauern) tauschen und ein einfaches Endspiel anstreben",
            ],
        )
    } else {
        t(
            lang,
            [
                "keep it safe and look to trade down",
                "conservar la ventaja y buscar cambios",
                "manter a vantagem e buscar trocas",
                "de garder l'avantage et de chercher des échanges",
                "den Vorteil sichern und Abtausch suchen",
            ],
        )
    };
    let tpl = if n == 1 {
        t(
            lang,
            [
                "{S} is up {n} point of material{what} — the plan is to {advice}.",
                "{S} tienen {n} punto de ventaja material{what}: el plan es {advice}.",
                "{S} têm {n} ponto de vantagem material{what}: o plano é {advice}.",
                "{S} ont {n} point d'avance matérielle{what} : le plan est {advice}.",
                "{S} hat {n} Punkt Materialvorteil{what} – der Plan: {advice}.",
            ],
        )
    } else {
        t(
            lang,
            [
                "{S} is up {n} points of material{what} — the plan is to {advice}.",
                "{S} tienen {n} puntos de ventaja material{what}: el plan es {advice}.",
                "{S} têm {n} pontos de vantagem material{what}: o plano é {advice}.",
                "{S} ont {n} points d'avance matérielle{what} : le plan est {advice}.",
                "{S} hat {n} Punkte Materialvorteil{what} – der Plan: {advice}.",
            ],
        )
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
                    // "un alfil" / "uma torre" / "un fou" / "ein Läufer".
                    Lang::Es | Lang::Pt | Lang::Fr | Lang::De => fill("{p_un}", &PieceRef::new(r).vars("p", lang)),
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
            Lang::Pt => format!(" ({} a mais)", join_list(&extra, lang)),
            Lang::Fr => format!(" ({} de plus)", join_list(&extra, lang)),
            Lang::De => format!(" ({} mehr)", join_list(&extra, lang)),
        }
    } else {
        let label = t(lang, ["extra", "de más", "a mais", "en plus", "zusätzlich"]);
        match lang {
            Lang::Fr => format!(" ({label} : {})", extra.join(", ")),
            _ => format!(" ({label}: {})", extra.join(", ")),
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
        return vec![t(
            lang,
            [
                "That position doesn't look valid — try setting it up again.",
                "Esa posición no parece válida: intenta colocarla de nuevo.",
                "Essa posição não parece válida: tente montá-la de novo.",
                "Cette position ne semble pas valide : essaie de la remettre en place.",
                "Diese Stellung scheint ungültig zu sein – versuch, sie neu aufzubauen.",
            ],
        )
        .to_string()];
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
            fill(
                t(
                    lang,
                    [
                        "Checkmate — {S} has won the game.",
                        "Jaque mate: {s} han ganado la partida.",
                        "Xeque-mate: {s} venceram a partida.",
                        "Échec et mat : {s} ont gagné la partie.",
                        "Schachmatt – {S} hat die Partie gewonnen.",
                    ],
                ),
                &kv(&[("S", &them), ("s", side_name(!stm, lang))]),
            ),
            t(
                lang,
                [
                    "Step back through the moves to see how the attack came together.",
                    "Repasa las jugadas para ver cómo se construyó el ataque.",
                    "Volte pelos lances para ver como o ataque foi construído.",
                    "Reviens sur les coups pour voir comment l'attaque s'est construite.",
                    "Geh die Züge zurück, um zu sehen, wie der Angriff entstanden ist.",
                ],
            )
            .to_string(),
            t(
                lang,
                [
                    "Notice which pieces covered the king's escape squares.",
                    "Fíjate en qué piezas cubrían las casillas de escape del rey.",
                    "Repare em quais peças cobriam as casas de fuga do rei.",
                    "Observe quelles pièces couvraient les cases de fuite du roi.",
                    "Achte darauf, welche Figuren die Fluchtfelder des Königs kontrolliert haben.",
                ],
            )
            .to_string(),
        ]
        .into_iter()
        .map(plain)
        .collect();
    }
    if pos.is_stalemate() {
        return [
            t(
                lang,
                [
                    "Stalemate — the game is a draw because the side to move has no legal moves but isn't in check.",
                    "Ahogado: la partida es tablas porque el bando que mueve no tiene jugadas legales y no está en jaque.",
                    "Afogamento: a partida termina empatada porque o lado que joga não tem lances legais, mas não está em xeque.",
                    "Pat : la partie est nulle, car le camp au trait n'est pas en échec mais n'a aucun coup légal.",
                    "Patt – die Partie ist remis, weil die Seite am Zug keinen legalen Zug hat, aber nicht im Schach steht.",
                ],
            ),
            t(
                lang,
                [
                    "When you're winning, always make sure the opponent has a legal move left!",
                    "Cuando vayas ganando, ¡asegúrate siempre de que tu rival tenga alguna jugada legal!",
                    "Quando estiver ganhando, garanta sempre que o adversário ainda tenha um lance legal!",
                    "Quand tu gagnes, vérifie toujours que l'adversaire a encore un coup légal !",
                    "Wenn du gewinnst, achte immer darauf, dass der Gegner noch einen legalen Zug hat!",
                ],
            ),
            t(
                lang,
                [
                    "Material doesn't matter anymore once it's stalemate.",
                    "Cuando hay ahogado, el material ya no importa.",
                    "No afogamento, o material não importa mais.",
                    "En cas de pat, le matériel ne compte plus.",
                    "Bei Patt spielt das Material keine Rolle mehr.",
                ],
            ),
        ]
        .into_iter()
        .map(|s| plain(s.to_string()))
        .collect();
    }
    if pos.is_insufficient_material() {
        return [
            t(
                lang,
                [
                    "Neither side has enough material to checkmate — it's a draw.",
                    "Ningún bando tiene material suficiente para dar mate: son tablas.",
                    "Nenhum lado tem material suficiente para dar mate: é empate.",
                    "Aucun camp n'a assez de matériel pour mater : c'est nulle.",
                    "Keine Seite hat genug Material zum Mattsetzen – remis.",
                ],
            ),
            t(
                lang,
                [
                    "Remember: a lone king plus a bishop or knight can't force mate.",
                    "Recuerda: un rey con solo un alfil o un caballo no puede forzar el mate.",
                    "Lembre-se: um rei com apenas um bispo ou um cavalo não consegue forçar o mate.",
                    "Rappel : un roi avec seulement un fou ou un cavalier ne peut pas forcer le mat.",
                    "Denk dran: Ein König mit nur einem Läufer oder Springer kann kein Matt erzwingen.",
                ],
            ),
            t(
                lang,
                [
                    "Try practising basic mates in the Endgames section.",
                    "Practica los mates básicos en la sección de Finales.",
                    "Pratique os mates básicos na seção de Finais.",
                    "Entraîne-toi aux mats de base dans la section Finales.",
                    "Übe die Grundmatts im Bereich Endspiele.",
                ],
            ),
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
                [
                    "{S} is in check and must deal with it first: move the king, block, or capture the checker.",
                    "{S} están en jaque y deben resolverlo primero: mover el rey, tapar el jaque o capturar la pieza que lo da.",
                    "{S} estão em xeque e precisam resolver isso primeiro: mover o rei, bloquear o xeque ou capturar a peça que o dá.",
                    "{S} sont en échec et doivent d'abord y parer : bouger le roi, interposer une pièce ou prendre la pièce qui donne échec.",
                    "{S} steht im Schach und muss das zuerst lösen: den König ziehen, dazwischenziehen oder die schachgebende Figur schlagen.",
                ],
            ),
            &kv(&[("S", &us)]),
        )));
    }
    if let Some(m) = tactics::mate_in_one(pos) {
        ideas.push(threat(fill(
            t(
                lang,
                [
                    "{S} has checkmate in one: {m}!",
                    "¡{S} tienen mate en una: {m}!",
                    "{S} têm mate em um: {m}!",
                    "{S} ont un mat en un coup : {m} !",
                    "{S} hat Matt in einem Zug: {m}!",
                ],
            ),
            &kv(&[("S", &us), ("m", &tactics::san(pos, &m))]),
        )));
    } else if let Some((swapped, m)) = tactics::null_move_mate_threat(pos) {
        let mv = tactics::san(&swapped, &m);
        let tpl = if tactics::is_back_rank_mate(&swapped, &m) {
            t(
                lang,
                [
                    "Watch out: {T} threatens {m}, checkmate on the back rank — {s} must defend.",
                    "Cuidado: {t} amenazan {m}, mate del pasillo; {s} deben defenderse.",
                    "Cuidado: {t} ameaçam {m}, mate na última fileira; {s} precisam se defender.",
                    "Attention : {t} menacent {m}, mat du couloir ; {s} doivent se défendre.",
                    "Vorsicht: {T} droht {m}, Grundreihenmatt – {s} muss sich verteidigen.",
                ],
            )
        } else {
            t(
                lang,
                [
                    "Watch out: {T} threatens {m}, checkmate — {s} must defend.",
                    "Cuidado: {t} amenazan {m}, jaque mate; {s} deben defenderse.",
                    "Cuidado: {t} ameaçam {m}, xeque-mate; {s} precisam se defender.",
                    "Attention : {t} menacent {m}, échec et mat ; {s} doivent se défendre.",
                    "Vorsicht: {T} droht {m}, Schachmatt – {s} muss sich verteidigen.",
                ],
            )
        };
        // English names the defender as a capitalized subject; the others use the plain name.
        let s = match lang {
            Lang::En => us.clone(),
            Lang::Es | Lang::Pt | Lang::Fr | Lang::De => side_name(stm, lang).to_string(),
        };
        ideas.push(threat(fill(tpl, &kv(&[("T", &them), ("t", side_name(!stm, lang)), ("m", &mv), ("s", &s)]))));
    }

    // 2. Hanging pieces (for the side to move these are opportunities; for the other, threats).
    let opp_hanging = tactics::hanging_pieces(b, !stm);
    if let Some(h) = opp_hanging.first() {
        let cap = tactics::best_capture_on(pos, h.square).map(|m| tactics::san(pos, &m));
        let mut v = PieceRef::on(h.role, h.square).colored(h.color).vars("p", lang);
        v.extend(kv(&[("s", side_name(stm, lang)), ("S", &us)]));
        let tpl = match (&cap, h.undefended) {
            (Some(_), true) => t(
                lang,
                [
                    "{P} is undefended — {S} can grab it with {c}.",
                    "{P} está indefens{p_o}: {s} pueden capturar{p_lo} con {c}.",
                    "{P} está indefes{p_o}: {s} podem capturá-l{p_o} com {c}.",
                    "{P} n'est pas défendu{p_o} : {s} peuvent {p_lo} prendre avec {c}.",
                    "{P} ist ungedeckt – {S} kann {p_lo} mit {c} schlagen.",
                ],
            ),
            (Some(_), false) => t(
                lang,
                [
                    "{P} is under-protected — {S} wins material with {c}.",
                    "{P} está mal defendid{p_o}: {s} ganan material con {c}.",
                    "{P} está mal defendid{p_o}: {s} ganham material com {c}.",
                    "{P} est mal défendu{p_o} : {s} gagnent du matériel avec {c}.",
                    "{P} ist zu schwach gedeckt – {S} gewinnt mit {c} Material.",
                ],
            ),
            (None, _) => t(
                lang,
                [
                    "{P} is loose — look for ways to attack it.",
                    "{P} está suelt{p_o}: busca maneras de atacar{p_lo}.",
                    "{P} está solt{p_o}: procure maneiras de atacá-l{p_o}.",
                    "{P} n'est pas protégé{p_o} : cherche comment {p_lo} attaquer.",
                    "{P} steht ungedeckt – such nach Wegen, {p_lo} anzugreifen.",
                ],
            ),
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
                [
                    "{P} is under attack — move it, defend it, or create a bigger threat.",
                    "{P} está atacad{p_o}: muéve{p_lo}, defiénde{p_lo} o crea una amenaza mayor.",
                    "{P} está sendo atacad{p_o}: mova-{p_lo}, defenda-{p_lo} ou crie uma ameaça maior.",
                    "{P} est attaqué{p_o} : déplace-{p_lo}, défends-{p_lo} ou crée une menace plus forte.",
                    "{P} wird angegriffen – zieh {p_lo} weg, deck {p_lo} oder schaffe eine größere Drohung.",
                ],
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
                        [
                            "{S} still has {n} minor pieces at home ({names}) — developing them is a priority.",
                            "{S} todavía tienen {n} piezas menores sin desarrollar ({names}): sacarlas es la prioridad.",
                            "{S} ainda têm {n} peças menores sem desenvolver ({names}): colocá-las em jogo é a prioridade.",
                            "{S} ont encore {n} pièces mineures non développées ({names}) : les sortir est la priorité.",
                            "{S} hat noch {n} unentwickelte Leichtfiguren ({names}) – sie zu entwickeln hat Vorrang.",
                        ],
                    ),
                    &kv(&[("S", &side_label(c, stm, lang)), ("n", &undeveloped.len().to_string()), ("names", &join_list(&names, lang))]),
                )));
                break;
            }
        }
    }
    for c in [stm, !stm] {
        let castled_rights = pos.castles().has(c, CastlingSide::KingSide) || pos.castles().has(c, CastlingSide::QueenSide);
        let king_ref = PieceRef::new(Role::King).colored(c);
        let king = king_ref.def(lang);
        if king_exposed(pos, c) {
            ideas.push(threat(fill(
                t(
                    lang,
                    [
                        "{K} looks exposed — attacking it (or tucking it away) is the key theme.",
                        "{K} parece expuesto: atacarlo (o ponerlo a salvo) es el tema clave.",
                        "{K} parece exposto: atacá-lo (ou colocá-lo em segurança) é o tema principal.",
                        "{K} semble exposé : l'attaquer (ou le mettre à l'abri) est le thème clé.",
                        "{K} wirkt ungeschützt – ihn anzugreifen (oder in Sicherheit zu bringen) ist das zentrale Thema.",
                    ],
                ),
                &kv(&[("K", &capitalize(&king))]),
            )));
            break;
        } else if castled_rights && pos.fullmoves().get() >= 6 && ph != Phase::Endgame {
            ideas.push(plain(fill(
                t(
                    lang,
                    [
                        "{S} hasn't castled yet — castling soon keeps the king safe and connects the rooks.",
                        "{S} aún no han enrocado: enrocar pronto pone el rey a salvo y conecta las torres.",
                        "{S} ainda não fizeram o roque: rocar logo deixa o rei seguro e conecta as torres.",
                        "{S} n'ont pas encore roqué : roquer bientôt met le roi à l'abri et relie les tours.",
                        "{S} hat noch nicht rochiert – eine baldige Rochade bringt den König in Sicherheit und verbindet die Türme.",
                    ],
                ),
                &kv(&[("S", &side_label(c, stm, lang))]),
            )));
            break;
        } else if pawn_shield_weak(b, c) && ph == Phase::Middlegame {
            let mut v = king_ref.vars("k", lang);
            v.extend(kv(&[("S", &side(c, lang))]));
            ideas.push(threat(fill(
                t(
                    lang,
                    [
                        "The pawn cover around {S}'s king is thin — keep an eye on attacks there.",
                        "La protección de peones de {k} es escasa: vigila los ataques por ese lado.",
                        "A proteção de peões {k_del} está fraca: fique de olho nos ataques por esse lado.",
                        "La protection de pions {k_del} est mince : surveille les attaques de ce côté.",
                        "Die Bauerndeckung {k_del} ist dünn – achte auf Angriffe auf dieser Seite.",
                    ],
                ),
                &v,
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
                        &["avance-o com apoio, e o adversário terá que bloqueá-lo com uma peça.", "peões passados devem avançar!", "ele pode virar dama se ninguém o parar."],
                        &["pousse-le avec du soutien, et l'adversaire devra le bloquer avec une pièce.", "les pions passés doivent avancer !", "il peut devenir une dame si personne ne l'arrête."],
                        &["schieb ihn mit Unterstützung vor, dann muss der Gegner ihn mit einer Figur blockieren.", "Freibauern müssen laufen!", "er kann zur Dame werden, wenn ihn niemand aufhält."],
                    ),
                );
                ideas.push(plain(fill(
                    t(
                        lang,
                        [
                            "{S} has a passed pawn on {sq} — {tail}",
                            "{S} tienen un peón pasado en {sq}: {tail}",
                            "{S} têm um peão passado em {sq}: {tail}",
                            "{S} ont un pion passé en {sq} : {tail}",
                            "{S} hat einen Freibauern auf {sq} – {tail}",
                        ],
                    ),
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
                    &["É um final: leve o rei para o centro, agora ele é uma peça forte.", "Hora do final: ative seu rei e crie um peão passado."],
                    &["C'est une finale : amène le roi vers le centre, c'est maintenant une pièce forte.", "C'est l'heure de la finale : active ton roi et crée un pion passé."],
                    &["Es ist ein Endspiel: Bring den König ins Zentrum – jetzt ist er eine starke Figur.", "Zeit fürs Endspiel: Aktiviere deinen König und schaffe einen Freibauern."],
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
                    [
                        "{S} controls the open {f}-file with a heavy piece — try to invade on the 7th rank.",
                        "{S} controlan la columna abierta {f} con una pieza pesada: intenta invadir por la séptima fila.",
                        "{S} controlam a coluna aberta {f} com uma peça pesada: tente invadir pela sétima fileira.",
                        "{S} contrôlent la colonne ouverte {f} avec une pièce lourde : essaie d'envahir la septième rangée.",
                        "{S} kontrolliert die offene {f}-Linie mit einer Schwerfigur – versuch, auf der siebten Reihe einzudringen.",
                    ],
                ),
                &kv(&[("S", &side(rooks_on[0], lang)), ("f", &letters[0])]),
            )));
        } else if rooks_on.is_empty() && !b.rooks().is_empty() {
            let tpl = if letters.len() > 1 {
                t(
                    lang,
                    [
                        "The {a}-file and {b}-file are open — whoever puts a rook there first gains an edge.",
                        "Las columnas {a} y {b} están abiertas: quien ponga primero una torre ahí gana ventaja.",
                        "As colunas {a} e {b} estão abertas: quem colocar uma torre ali primeiro ganha vantagem.",
                        "Les colonnes {a} et {b} sont ouvertes : celui qui y place une tour en premier prend l'avantage.",
                        "Die {a}- und die {b}-Linie sind offen – wer zuerst einen Turm dorthin stellt, ist im Vorteil.",
                    ],
                )
            } else {
                t(
                    lang,
                    [
                        "The {a}-file is open — whoever puts a rook there first gains an edge.",
                        "La columna {a} está abierta: quien ponga primero una torre ahí gana ventaja.",
                        "A coluna {a} está aberta: quem colocar uma torre ali primeiro ganha vantagem.",
                        "La colonne {a} est ouverte : celui qui y place une tour en premier prend l'avantage.",
                        "Die {a}-Linie ist offen – wer zuerst einen Turm dorthin stellt, ist im Vorteil.",
                    ],
                )
            };
            let second = letters.get(1).cloned().unwrap_or_default();
            ideas.push(plain(fill(tpl, &kv(&[("a", &letters[0]), ("b", &second)]))));
        }
    }

    // 8. Pawn weaknesses.
    for c in [!stm, stm] {
        let iso = isolated_pawns(b, c);
        let dbl = doubled_files(b, c);
        if let Some(sq) = iso.first().filter(|_| ph != Phase::Opening) {
            let pawn = PieceRef::on(Role::Pawn, *sq).colored(c).def(lang);
            ideas.push(plain(fill(
                t(
                    lang,
                    [
                        "{P} is isolated — no pawn can defend it, so it's a target.",
                        "{P} está aislado: ningún peón puede defenderlo, así que es un objetivo.",
                        "{P} está isolado: nenhum peão pode defendê-lo, então ele é um alvo.",
                        "{P} est isolé : aucun pion ne peut le défendre, c'est donc une cible.",
                        "{P} ist isoliert – kein Bauer kann ihn decken, also ist er ein Angriffsziel.",
                    ],
                ),
                &kv(&[("P", &capitalize(&pawn))]),
            )));
            break;
        }
        if let Some(f) = dbl.first().filter(|_| ph != Phase::Opening) {
            ideas.push(plain(fill(
                t(
                    lang,
                    [
                        "{S} has doubled pawns on the {f}-file, a small long-term weakness.",
                        "{S} tienen peones doblados en la columna {f}, una pequeña debilidad a largo plazo.",
                        "{S} têm peões dobrados na coluna {f}, uma pequena fraqueza de longo prazo.",
                        "{S} ont des pions doublés sur la colonne {f}, une petite faiblesse à long terme.",
                        "{S} hat einen Doppelbauern auf der {f}-Linie, eine kleine langfristige Schwäche.",
                    ],
                ),
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
                [
                    "White has more pawns in the center — Black should challenge it with pawn breaks like ...c5 or ...d5.",
                    "Las blancas tienen más peones en el centro: las negras deberían desafiarlo con rupturas como ...c5 o ...d5.",
                    "As brancas têm mais peões no centro: as pretas deveriam desafiá-lo com rupturas como ...c5 ou ...d5.",
                    "Les Blancs ont plus de pions au centre : les Noirs devraient le contester avec des poussées comme ...c5 ou ...d5.",
                    "Weiß hat mehr Bauern im Zentrum – Schwarz sollte es mit Hebeln wie ...c5 oder ...d5 angreifen.",
                ],
            )
        } else if bl > w {
            t(
                lang,
                [
                    "Black has more central pawns — White should fight back for the center.",
                    "Las negras tienen más peones centrales: las blancas deberían luchar por el centro.",
                    "As pretas têm mais peões centrais: as brancas deveriam lutar pelo centro.",
                    "Les Noirs ont plus de pions centraux : les Blancs devraient se battre pour le centre.",
                    "Schwarz hat mehr Zentrumsbauern – Weiß sollte um das Zentrum kämpfen.",
                ],
            )
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
                    &[
                        "Princípios de abertura: controle o centro, desenvolva cavalos e bispos e faça o roque cedo.",
                        "O básico: desenvolva uma peça nova a cada lance e não mova a mesma peça duas vezes sem motivo.",
                    ],
                    &[
                        "Principes d'ouverture : contrôle le centre, développe cavaliers et fous, et roque tôt.",
                        "Les bases : développe une nouvelle pièce à chaque coup et ne joue pas deux fois la même pièce sans raison.",
                    ],
                    &[
                        "Eröffnungsprinzipien: Kontrolliere das Zentrum, entwickle Springer und Läufer und rochiere früh.",
                        "Die Grundlagen: Entwickle mit jedem Zug eine neue Figur und zieh dieselbe Figur nicht ohne Grund zweimal.",
                    ],
                ),
            )
        };
        ideas.push(plain(text.to_string()));
    }

    // Keep 3..=6, de-duplicated.
    let mut seen = std::collections::HashSet::new();
    ideas.retain(|i| seen.insert(i.text.clone()));
    let fillers = fillers(lang);
    let mut i = 0;
    while ideas.len() < 3 && i < fillers.len() {
        ideas.push(Idea { text: fillers[i].to_string(), threat: false });
        i += 1;
    }
    ideas.truncate(6);
    ideas
}

/// General advice used to pad short idea lists.
fn fillers(lang: Lang) -> &'static [&'static str] {
    tl(
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
        &[
            "Antes de cada lance, procure xeques, capturas e ameaças, para os dois lados.",
            "Encontre sua peça menos ativa e procure uma casa melhor para ela.",
            "Pergunte-se: o que o adversário quer fazer a seguir?",
        ],
        &[
            "Avant chaque coup, cherche les échecs, les captures et les menaces, pour les deux camps.",
            "Trouve ta pièce la moins active et cherche-lui une meilleure case.",
            "Demande-toi : que veut faire l'adversaire ensuite ?",
        ],
        &[
            "Prüfe vor jedem Zug Schachgebote, Schlagzüge und Drohungen – für beide Seiten.",
            "Finde deine am wenigsten aktive Figur und such ihr ein besseres Feld.",
            "Frag dich: Was will der Gegner als Nächstes tun?",
        ],
    )
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

    #[test]
    fn new_language_ideas() {
        let cases = [
            (
                Lang::Pt,
                "O material está igualado.",
                "O cavalo preto em d5 está indefeso: as brancas podem capturá-lo com Rxd5.",
                "As brancas têm 2 pontos de vantagem material",
                "As brancas têm mate em um: Ra8#!",
                "A torre branca em a1 está indefesa",
            ),
            (
                Lang::Fr,
                "Le matériel est égal.",
                "Le cavalier noir en d5 n'est pas défendu : les Blancs peuvent le prendre avec Rxd5.",
                "Les Blancs ont 2 points d'avance matérielle",
                "Les Blancs ont un mat en un coup : Ra8# !",
                "La tour blanche en a1 n'est pas défendue",
            ),
            (
                Lang::De,
                "Das Material ist ausgeglichen.",
                "Der schwarze Springer auf d5 ist ungedeckt – Weiß kann ihn mit Rxd5 schlagen.",
                "Weiß hat 2 Punkte Materialvorteil",
                "Weiß hat Matt in einem Zug: Ra8#!",
                "Der weiße Turm auf a1 ist ungedeckt",
            ),
        ];
        for (lang, level, knight, material, mate, rook) in cases {
            let ideas = describe_position("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", lang);
            assert!(ideas.iter().any(|i| i == level), "{lang}: {ideas:?}");
            let ideas = describe_position("4k3/8/8/3n4/8/8/8/3RK3 w - - 0 1", lang);
            assert!(ideas.iter().any(|i| i.starts_with(knight)), "{lang}: {ideas:?}");
            assert!(ideas.iter().any(|i| i.starts_with(material)), "{lang}: {ideas:?}");
            let ideas = describe_position("6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1", lang);
            assert!(ideas.iter().any(|i| i.contains(mate)), "{lang}: {ideas:?}");
            let ideas = describe_position("4k3/8/8/8/3b4/8/8/R3K3 b - - 0 1", lang);
            assert!(ideas.iter().any(|i| i.contains(rook)), "{lang}: {ideas:?}");
            for i in &ideas {
                assert!(!i.contains(" the ") && !i.contains("White") && !i.contains("Black"), "{lang}: {i}");
            }
            assert!(!describe_position("garbage", lang)[0].contains("valid —"), "{lang}");
        }
    }

    /// Every language produces 3..=6 non-empty, fully filled ideas for a range of positions
    /// (opening, middlegame, endgame, check, mate, stalemate, insufficient material).
    #[test]
    fn ideas_never_empty_in_any_language() {
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 6",
            "4k3/8/8/3n4/8/8/8/3RK3 w - - 0 1",
            "6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1",
            "6k1/5ppp/8/8/8/8/5PPP/4R1K1 b - - 0 1",
            "4k3/8/8/8/3b4/8/8/R3K3 b - - 0 1",
            "8/5k2/8/3P4/8/8/5K2/8 w - - 0 40",
            "R5k1/5ppp/8/8/8/8/8/6K1 b - - 1 1",
            "7k/5Q2/6K1/8/8/8/8/8 b - - 0 1",
            "8/8/4k3/8/8/4K3/8/8 w - - 0 1",
            "r3k2r/pp3ppp/2n5/3q4/8/2N5/PP3PPP/R2QK2R w KQkq - 0 12",
            "4k3/8/8/8/8/8/4q3/4K3 w - - 0 1",
        ];
        for lang in Lang::ALL {
            for fen in fens {
                let ideas = describe_position(fen, lang);
                assert!((1..=6).contains(&ideas.len()), "{lang} {fen}: {ideas:?}");
                for i in &ideas {
                    assert!(!i.trim().is_empty(), "{lang} {fen}");
                    assert!(!i.contains('{') && !i.contains('}'), "{lang} {fen}: {i}");
                }
            }
            assert!(fillers(lang).len() >= 3);
            assert_eq!(describe_position("garbage", lang).len(), 1);
        }
    }
}
