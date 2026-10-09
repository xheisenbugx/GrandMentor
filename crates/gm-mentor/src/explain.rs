//! Rule-based move explanations in the style of a friendly human coach, in every [`Lang`].
//!
//! Positive reasons are verb phrases ("develops the knight toward the center" / "desarrolla el
//! caballo hacia el centro") joined under a subject ("This" / "Esta jugada"); problems are full
//! sentences. Pieces are rendered through [`PieceRef::vars`] so Spanish articles, contractions
//! and adjectives agree with each piece's gender.

use gm_content::words::{fill, kv, piece_plural, PieceRef};
use gm_content::Lang;
use gm_engine::Score;
use shakmaty::{Chess, Color, Position, Role, Square};

use crate::phrase::Picker;
use crate::tactics::{value, LineKind, MoveFacts};
use crate::{tactics, MoveContext};

/// Move quality bucket derived from the classification string (or from evals when missing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cat {
    Brilliant,
    Great,
    Best,
    Good,
    Book,
    Forced,
    Inaccuracy,
    Mistake,
    Miss,
    Blunder,
}

impl Cat {
    fn is_bad(self) -> bool {
        matches!(self, Cat::Inaccuracy | Cat::Mistake | Cat::Miss | Cat::Blunder)
    }
}

/// Score in centipawns from `side`'s point of view; mates are mapped to +-100000 (nearer = bigger).
pub(crate) fn pov_cp(score: Score, side: Color) -> i32 {
    let white = match score {
        Score::Cp(c) => c.clamp(-50_000, 50_000),
        Score::Mate(m) if m > 0 => 100_000 - m.min(1000),
        Score::Mate(m) if m < 0 => -100_000 - m.max(-1000),
        Score::Mate(_) => 0,
    };
    match side {
        Color::White => white,
        Color::Black => -white,
    }
}

/// Mate distance for `side` (positive = `side` mates).
pub(crate) fn pov_mate(score: Score, side: Color) -> Option<i32> {
    match score {
        Score::Mate(m) if m != 0 => Some(if side == Color::White { m } else { -m }),
        _ => None,
    }
}

fn category(cls: &str, drop: i32) -> Cat {
    match cls.trim().to_ascii_lowercase().as_str() {
        "brilliant" => Cat::Brilliant,
        "great" => Cat::Great,
        "best" => Cat::Best,
        "excellent" | "good" => Cat::Good,
        "book" => Cat::Book,
        "forced" => Cat::Forced,
        "inaccuracy" => Cat::Inaccuracy,
        "mistake" => Cat::Mistake,
        "miss" => Cat::Miss,
        "blunder" => Cat::Blunder,
        _ => match drop {
            d if d <= 10 => Cat::Best,
            d if d <= 50 => Cat::Good,
            d if d <= 100 => Cat::Inaccuracy,
            d if d <= 250 => Cat::Mistake,
            _ => Cat::Blunder,
        },
    }
}

/// Pick the template list for `lang` (English, Spanish, Portuguese, French, German).
fn t<'a>(lang: Lang, en: &'a [&'a str], es: &'a [&'a str], pt: &'a [&'a str], fr: &'a [&'a str], de: &'a [&'a str]) -> &'a [&'a str] {
    match lang {
        Lang::En => en,
        Lang::Es => es,
        Lang::Pt => pt,
        Lang::Fr => fr,
        Lang::De => de,
    }
}

/// Pick one string for `lang` from `[en, es, pt, fr, de]`.
fn l5(lang: Lang, s: [&'static str; 5]) -> &'static str {
    match lang {
        Lang::En => s[0],
        Lang::Es => s[1],
        Lang::Pt => s[2],
        Lang::Fr => s[3],
        Lang::De => s[4],
    }
}

/// Variables for a piece standing on `sq` (the king is just "the king").
fn pv(prefix: &str, role: Role, sq: Square, lang: Lang) -> Vec<(String, String)> {
    PieceRef::on_or_king(role, sq).vars(prefix, lang)
}

/// Variables for a piece type without a square ("the knight" / "el caballo").
fn rv(prefix: &str, role: Role, lang: Lang) -> Vec<(String, String)> {
    PieceRef::new(role).vars(prefix, lang)
}

fn cat(mut a: Vec<(String, String)>, b: Vec<(String, String)>) -> Vec<(String, String)> {
    a.extend(b);
    a
}

/// Positive verb phrases ("develops the knight ...") describing what a move achieves, best first.
/// German phrases are written for verb-second order after the subject ("Dieser Zug entwickelt
/// den Springer ..."), so direct objects use the accusative keys (`{p_acc}`, `{p_un_acc}`).
pub(crate) fn positive_reasons(f: &MoveFacts, p: &Picker, lang: Lang) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if f.is_mate {
        out.push(l5(lang, ["delivers checkmate", "da jaque mate", "dá xeque-mate", "donne échec et mat", "setzt matt"]).to_string());
        return out;
    }
    if f.fork.len() >= 2 {
        let (a, b) = (f.fork[0], f.fork[1]);
        let tpl = p.pick(
            11,
            t(
                lang,
                &[
                    "forks {a} and {b} — they can't both be saved",
                    "attacks {a} and {b} at the same time, a classic fork",
                    "creates a fork, hitting {a} and {b} at once",
                ],
                &[
                    "hace un ataque doble {a_al} y {b_al}: no se pueden salvar las dos piezas",
                    "ataca a la vez {a_al} y {b_al}, un ataque doble de manual",
                    "crea un ataque doble sobre {a} y {b}",
                ],
                &[
                    "faz um garfo em {a} e {b}: não dá para salvar as duas peças",
                    "ataca {a} e {b} ao mesmo tempo, um garfo clássico",
                    "cria um garfo, atacando {a} e {b} de uma vez",
                ],
                &[
                    "fait une fourchette sur {a} et {b} : impossible de sauver les deux",
                    "attaque à la fois {a} et {b}, une fourchette classique",
                    "crée une fourchette qui touche {a} et {b} en même temps",
                ],
                &[
                    "macht eine Gabel gegen {a_acc} und {b_acc} – beide sind nicht zu retten",
                    "greift gleichzeitig {a_acc} und {b_acc} an, eine klassische Gabel",
                    "stellt eine Gabel: {a} und {b} sind gleichzeitig angegriffen",
                ],
            ),
        );
        out.push(fill(tpl, &cat(pv("a", a.1, a.0, lang), pv("b", b.1, b.0, lang))));
    }
    if let Some(m) = &f.stops_mate {
        let tpl = p.pick(
            25,
            t(
                lang,
                &["stops the threat of {m}", "defends against {m}, which was threatening mate", "shuts down the mating threat {m}"],
                &["frena la amenaza de {m}", "se defiende de {m}, que amenazaba mate", "neutraliza la amenaza de mate {m}"],
                &["impede a ameaça de {m}", "se defende de {m}, que ameaçava mate", "neutraliza a ameaça de mate {m}"],
                &["pare la menace {m}", "se défend contre {m}, qui menaçait de faire mat", "neutralise la menace de mat {m}"],
                &["wehrt die Drohung {m} ab", "verteidigt sich gegen das drohende Matt mit {m}", "schaltet die Mattdrohung {m} aus"],
            ),
        );
        out.push(fill(tpl, &kv(&[("m", m)])));
    }
    if let Some(cap) = f.captured {
        if f.wins_material() {
            let v = rv("c", cap, lang);
            let tpl = if f.material_lead < 0 {
                p.pick(
                    12,
                    t(
                        lang,
                        &["wins back the {c_n}", "recaptures the {c_n} and restores the balance", "takes back the {c_n}"],
                        &["recupera material capturando {c}", "captura {c} y restablece el equilibrio", "recupera material al llevarse {c}"],
                        &["recupera material capturando {c}", "captura {c} e restabelece o equilíbrio", "recupera material ao tomar {c}"],
                        &["regagne du matériel en prenant {c}", "prend {c} et rétablit l'équilibre", "récupère du matériel en capturant {c}"],
                        &["gewinnt {c_acc} zurück", "schlägt {c_acc} und stellt das Gleichgewicht wieder her", "holt sich {c_acc} zurück"],
                    ),
                )
            } else if !f.recapturable && f.capture_net >= value(cap) - 10 {
                p.pick(
                    12,
                    t(
                        lang,
                        &["wins {c_un} for free", "picks up a free {c_n}", "grabs the undefended {c_n}"],
                        &["gana {c_un} gratis", "se lleva {c_un} gratis", "captura {c} indefens{c_o}"],
                        &["ganha {c_un} de graça", "leva {c_un} de graça", "captura {c} indefes{c_o}"],
                        &["gagne {c_un} gratuitement", "empoche {c_un} gratuitement", "prend {c}, qui n'était pas défendu{c_o}"],
                        &["gewinnt {c_un_acc} umsonst", "schnappt sich {c_un_acc} gratis", "kassiert {c_acc}, ohne etwas dafür zu geben"],
                    ),
                )
            } else if f.capture_net >= value(cap) - 30 {
                l5(lang, ["wins {c_un}", "gana {c_un}", "ganha {c_un}", "gagne {c_un}", "gewinnt {c_un_acc}"])
            } else {
                l5(
                    lang,
                    [
                        "comes out ahead in the exchange",
                        "sale ganando en el intercambio",
                        "sai ganhando na troca",
                        "sort gagnant de l'échange",
                        "geht aus dem Abtausch mit Gewinn hervor",
                    ],
                )
            };
            out.push(fill(tpl, &v));
        }
    }
    if let Some(l) = f.lines.first() {
        let v = cat(pv("f", l.front.1, l.front.0, lang), pv("b", l.behind.1, l.behind.0, lang));
        let tpl = match l.kind {
            LineKind::Pin => p.pick(
                13,
                t(
                    lang,
                    &["pins {f} to {b}", "sets up a pin: {f} can't move without exposing {b}"],
                    &["clava {f} contra {b}", "crea una clavada: {f} no puede moverse sin dejar expuest{b_o} {b}"],
                    &["crava {f} contra {b}", "cria uma cravada: {f} não pode se mover sem deixar expost{b_o} {b}"],
                    &["cloue {f} sur {b}", "crée un clouage : {f} ne peut pas bouger sans exposer {b}"],
                    &["fesselt {f_acc} an {b_acc}", "baut eine Fesselung auf: {f} kann nicht ziehen, ohne {b_acc} preiszugeben"],
                ),
            ),
            LineKind::Skewer => p.pick(
                14,
                t(
                    lang,
                    &["skewers {f} and {b} behind it", "lines up a skewer through {f} to {b}"],
                    &["hace una enfilada {f_al}, con {b} detrás", "ataca en enfilada {f_al} y {b_al}, que está detrás"],
                    &["faz um espeto: ataca {f}, com {b} logo atrás", "ataca em espeto {f} e {b}, que está atrás"],
                    &["fait une enfilade sur {f}, avec {b} derrière", "aligne {f} et {b} en enfilade"],
                    &["spießt {f_acc} auf – dahinter steht {b}", "stellt einen Spieß: erst {f}, dahinter {b}"],
                ),
            ),
        };
        out.push(fill(tpl, &v));
    }
    if let Some(r) = f.promotion {
        let tpl = l5(
            lang,
            [
                "promotes the pawn to {r_un}",
                "corona el peón y consigue {r_un}",
                "promove o peão e consegue {r_un}",
                "promeut le pion en {r_n}",
                "wandelt den Bauern in {r_un_acc} um",
            ],
        );
        out.push(fill(tpl, &rv("r", r, lang)));
    }
    if let Some(m) = &f.threatens_mate {
        let tpl = p.pick(
            15,
            t(
                lang,
                &["threatens {m} with checkmate", "sets up a mating threat ({m})", "creates the threat of {m}, checkmate"],
                &["amenaza mate con {m}", "prepara una amenaza de mate ({m})", "crea la amenaza de {m}, jaque mate"],
                &["ameaça mate com {m}", "prepara uma ameaça de mate ({m})", "cria a ameaça de {m}, xeque-mate"],
                &["menace de faire mat avec {m}", "prépare une menace de mat ({m})", "crée la menace {m}, échec et mat"],
                &["droht Matt mit {m}", "baut eine Mattdrohung auf ({m})", "stellt die Drohung {m} auf – Schachmatt"],
            ),
        );
        out.push(fill(tpl, &kv(&[("m", m)])));
    }
    if f.is_trade() {
        if f.material_lead >= 2 {
            let tpl = p.pick(
                17,
                t(
                    lang,
                    &[
                        "trades pieces — a great idea when you're ahead in material",
                        "simplifies the position, which helps the side that's ahead",
                    ],
                    &[
                        "cambia piezas, una gran idea cuando vas por delante en material",
                        "simplifica la posición, algo que favorece al bando que va ganando",
                    ],
                    &[
                        "troca peças, uma ótima ideia quando você está com mais material",
                        "simplifica a posição, o que favorece o lado que está na frente",
                    ],
                    &[
                        "échange des pièces, une très bonne idée quand tu as plus de matériel",
                        "simplifie la position, ce qui aide le camp qui mène",
                    ],
                    &[
                        "tauscht Figuren – eine gute Idee, wenn du materiell vorne liegst",
                        "vereinfacht die Stellung, was der stärkeren Seite hilft",
                    ],
                ),
            );
            out.push(tpl.to_string());
        } else {
            let ns = match f.captured {
                Some(r) => piece_plural(r, lang),
                None => l5(lang, ["pieces", "piezas", "peças", "pièces", "Figuren"]),
            };
            let tpl = p.pick(
                18,
                t(
                    lang,
                    &["keeps the material balanced by trading {ns}", "recaptures and keeps material level", "completes a fair trade of {ns}"],
                    &["mantiene el equilibrio material cambiando {ns}", "recaptura y mantiene el material igualado", "completa un cambio justo de {ns}"],
                    &["mantém o equilíbrio material trocando {ns}", "recaptura e mantém o material igualado", "completa uma troca justa de {ns}"],
                    &["garde l'équilibre matériel en échangeant des {ns}", "reprend et garde l'égalité matérielle", "conclut un échange équitable de {ns}"],
                    &["hält durch den Tausch der {ns} das Material ausgeglichen", "schlägt zurück und hält das Material ausgeglichen", "schließt einen fairen Tausch der {ns} ab"],
                ),
            );
            out.push(fill(tpl, &kv(&[("ns", ns)])));
        }
    }
    if let Some((sq, r)) = f.new_threat {
        let v = pv("p", r, sq, lang);
        let tpl = if r == Role::Queen {
            l5(
                lang,
                ["attacks the queen on {sq}", "ataca a la dama en {sq}", "ataca a dama em {sq}", "attaque la dame en {sq}", "greift die Dame auf {sq} an"],
            )
        } else {
            p.pick(
                16,
                t(
                    lang,
                    &["attacks {p}, which is now in trouble", "goes after {p}", "puts pressure on {p}"],
                    &["ataca {p_al}, que ahora está en apuros", "persigue {p_al}", "presiona {p_al}"],
                    &["ataca {p}, que agora está em apuros", "vai atrás {p_del}", "pressiona {p}"],
                    &["attaque {p}, qui est maintenant en difficulté", "s'en prend {p_al}", "met la pression sur {p}"],
                    &["greift {p_acc} an – {p_er} gerät jetzt in Schwierigkeiten", "nimmt {p_acc} ins Visier", "setzt {p_acc} unter Druck"],
                ),
            )
        };
        out.push(fill(tpl, &cat(v, kv(&[("sq", &sq.to_string())]))));
    }
    if let Some((sq, r)) = f.defends.filter(|_| f.captured.is_none()) {
        let tpl = p.pick(
            26,
            t(
                lang,
                &["protects {p}", "takes care of {p}, which was under attack"],
                &["defiende {p}", "protege {p}, que estaba atacad{p_o}"],
                &["defende {p}", "protege {p}, que estava sob ataque"],
                &["défend {p}", "protège {p}, qui était attaqué{p_o}"],
                &["deckt {p_acc}", "schützt {p_acc} vor dem Angriff"],
            ),
        );
        out.push(fill(tpl, &pv("p", r, sq, lang)));
    }
    if f.saves_piece {
        let tpl = p.pick(
            19,
            t(
                lang,
                &["moves {p} out of danger", "rescues the attacked {p_n}", "gets {p} to safety"],
                &["saca {p} del peligro", "rescata {p} atacad{p_o}", "pone {p} a salvo"],
                &["tira {p} do perigo", "salva {p} atacad{p_o}", "coloca {p} em segurança"],
                &["met {p} hors de danger", "sauve {p} attaqué{p_o}", "met {p} à l'abri"],
                &["bringt {p_acc} aus der Gefahrenzone", "rettet {p_acc} vor dem Angriff", "bringt {p_acc} in Sicherheit"],
            ),
        );
        out.push(fill(tpl, &rv("p", f.role, lang)));
    }
    if f.castle.is_some() {
        let tpl = p.pick(
            20,
            t(
                lang,
                &[
                    "castles, tucking the king into safety and connecting the rooks",
                    "gets the king safe and brings a rook toward the center",
                    "castles — king safety first, and the rooks can now work together",
                ],
                &[
                    "enroca, pone el rey a salvo y conecta las torres",
                    "pone el rey a salvo y acerca una torre al centro",
                    "enroca: primero la seguridad del rey, y ahora las torres pueden colaborar",
                ],
                &[
                    "é um roque que coloca o rei em segurança e conecta as torres",
                    "coloca o rei em segurança e aproxima uma torre do centro",
                    "é um roque: primeiro a segurança do rei, e agora as torres podem trabalhar juntas",
                ],
                &[
                    "met le roi à l'abri par le roque et relie les tours",
                    "met le roi en sécurité et rapproche une tour du centre",
                    "met le roi à l'abri : la sécurité d'abord, et les tours peuvent maintenant coopérer",
                ],
                &[
                    "ist eine Rochade, die den König in Sicherheit bringt und die Türme verbindet",
                    "bringt den König in Sicherheit und einen Turm Richtung Zentrum",
                    "ist eine Rochade – Königssicherheit zuerst, und jetzt arbeiten die Türme zusammen",
                ],
            ),
        );
        out.push(tpl.to_string());
    }
    if f.develops {
        let tpl = p.pick(
            21,
            t(
                lang,
                &["develops {p} toward the center", "brings {p} into the game", "gets another piece into play"],
                &["desarrolla {p} hacia el centro", "pone {p} en juego", "pone en juego otra pieza"],
                &["desenvolve {p} em direção ao centro", "coloca {p} em jogo", "coloca mais uma peça em jogo"],
                &["développe {p} vers le centre", "fait entrer {p} dans le jeu", "met une pièce de plus en jeu"],
                &["entwickelt {p_acc} Richtung Zentrum", "bringt {p_acc} ins Spiel", "bringt eine weitere Figur ins Spiel"],
            ),
        );
        out.push(fill(tpl, &rv("p", f.role, lang)));
    }
    if f.center {
        let tpl = if f.role == Role::Pawn {
            p.pick(
                22,
                t(
                    lang,
                    &["claims space in the center", "stakes a claim in the center", "grabs a big share of the center"],
                    &["gana espacio en el centro", "reclama su parte del centro", "se hace con buena parte del centro"],
                    &["ganha espaço no centro", "reivindica sua parte do centro", "toma boa parte do centro"],
                    &["gagne de l'espace au centre", "revendique sa part du centre", "s'empare d'une bonne partie du centre"],
                    &["gewinnt Raum im Zentrum", "beansprucht seinen Teil des Zentrums", "sichert sich einen großen Teil des Zentrums"],
                ),
            )
        } else {
            p.pick(
                23,
                t(
                    lang,
                    &["controls important central squares", "eyes the center"],
                    &["controla casillas centrales importantes", "apunta al centro"],
                    &["controla casas centrais importantes", "mira o centro"],
                    &["contrôle des cases centrales importantes", "vise le centre"],
                    &["kontrolliert wichtige Zentrumsfelder", "zielt aufs Zentrum"],
                ),
            )
        };
        out.push(tpl.to_string());
    }
    if f.gives_check && out.is_empty() {
        let tpl = p.pick(
            24,
            t(
                lang,
                &["gives check and keeps the opponent busy", "checks the king and keeps the initiative"],
                &["da jaque y mantiene ocupado al rival", "da jaque al rey y mantiene la iniciativa"],
                &["dá xeque e mantém o adversário ocupado", "dá xeque no rei e mantém a iniciativa"],
                &["donne échec et occupe l'adversaire", "met le roi en échec et garde l'initiative"],
                &["gibt Schach und hält den Gegner beschäftigt", "bietet Schach und behält die Initiative"],
            ),
        );
        out.push(tpl.to_string());
    }
    out
}

/// Negative full sentences describing what's wrong with the played move. Sentences that talk
/// about the move itself start with [`subject`] so callers can substitute the SAN.
fn problems(ctx: &MoveContext, f: &MoveFacts, best: Option<&MoveFacts>, cat_: Cat, p: &Picker, lang: Lang) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mover = f.mover;
    if let Some((m, back_rank)) = &f.allows_mate {
        let tpl = if *back_rank {
            p.pick(
                31,
                t(
                    lang,
                    &[
                        "This allows a back-rank mate with {m} — the king has no escape squares.",
                        "Careful: {m} is now a back-rank checkmate, since the king is boxed in by its own pawns.",
                    ],
                    &[
                        "Esta jugada permite un mate del pasillo con {m}: el rey no tiene casillas de escape.",
                        "Cuidado: ahora {m} es mate del pasillo, porque el rey está encerrado por sus propios peones.",
                    ],
                    &[
                        "Este lance permite um mate na última fileira com {m}: o rei não tem casas de fuga.",
                        "Cuidado: agora {m} é mate na última fileira, porque o rei está preso pelos próprios peões.",
                    ],
                    &[
                        "Ce coup permet un mat du couloir avec {m} : le roi n'a aucune case de fuite.",
                        "Attention : {m} est maintenant un mat du couloir, car le roi est enfermé par ses propres pions.",
                    ],
                    &[
                        "Dieser Zug erlaubt ein Grundreihenmatt mit {m}: Der König hat keine Fluchtfelder.",
                        "Vorsicht: {m} ist jetzt ein Grundreihenmatt, weil der König von den eigenen Bauern eingesperrt ist.",
                    ],
                ),
            )
        } else {
            p.pick(
                32,
                t(
                    lang,
                    &["This allows {m}, checkmate!", "Ouch — this lets {m} deliver checkmate."],
                    &["Esta jugada permite {m}, ¡jaque mate!", "¡Ay! Esta jugada permite que {m} dé jaque mate."],
                    &["Este lance permite {m}, xeque-mate!", "Ai! Este lance deixa {m} dar xeque-mate."],
                    &["Ce coup permet {m}, échec et mat !", "Aïe ! Ce coup permet {m}, qui fait échec et mat."],
                    &["Dieser Zug erlaubt {m}, Schachmatt!", "Autsch! Dieser Zug erlaubt {m} – und das ist matt."],
                ),
            )
        };
        out.push(fill(tpl, &kv(&[("m", m)])));
        return out;
    }
    if let Some(m) = &f.missed_mate {
        let tpl = p.pick(
            33,
            t(
                lang,
                &["You had checkmate in one with {m}!", "There was a mate in one: {m}!"],
                &["¡Tenías mate en una con {m}!", "¡Había mate en una: {m}!"],
                &["Você tinha mate em um com {m}!", "Havia um mate em um: {m}!"],
                &["Tu avais un mat en un coup avec {m} !", "Il y avait un mat en un coup : {m} !"],
                &["Du hattest Matt in einem Zug mit {m}!", "Hier gab es ein Matt in einem Zug: {m}!"],
            ),
        );
        out.push(fill(tpl, &kv(&[("m", m)])));
        return out;
    }
    if let (Some(before), after) = (pov_mate(ctx.eval_before, mover), pov_mate(ctx.eval_after, mover)) {
        if before > 0 && after.map(|a| a <= 0).unwrap_or(true) {
            let tpl = p.pick(
                34,
                t(
                    lang,
                    &["This lets a forced checkmate slip away.", "There was a forced mate here, and this move misses it."],
                    &["Esta jugada deja escapar un mate forzado.", "Aquí había un mate forzado, y esta jugada lo deja pasar."],
                    &["Este lance deixa escapar um mate forçado.", "Havia um mate forçado aqui, e este lance o deixa passar."],
                    &["Ce coup laisse échapper un mat forcé.", "Il y avait un mat forcé ici, et ce coup le laisse passer."],
                    &["Dieser Zug lässt ein erzwungenes Matt aus.", "Hier gab es ein erzwungenes Matt, und dieser Zug verpasst es."],
                ),
            );
            out.push(tpl.to_string());
        }
    }
    if f.is_stalemate && f.material_lead > 0 {
        out.push(
            l5(
                lang,
                [
                    "This is stalemate — a winning position thrown away as a draw!",
                    "Esta jugada es ahogado: ¡una posición ganada que se convierte en tablas!",
                    "Este lance é afogamento: uma posição ganha que vira empate!",
                    "Ce coup fait pat : une position gagnante qui devient nulle !",
                    "Dieser Zug ist Patt – eine gewonnene Stellung wird als Remis verschenkt!",
                ],
            )
            .to_string(),
        );
        return out;
    }
    // A capture that loses more than any other hanging piece is the real problem (e.g. Qxf7+?? Kxf7
    // is about the queen, not about some pawn left hanging elsewhere).
    let capture_loss = if f.captured.is_some() { -f.capture_net } else { 0 };
    if let Some((h, cap, was)) = f.hangs.first().filter(|(h, _, _)| capture_loss <= h.gain) {
        if h.gain >= 90 {
            let v = pv("p", h.role, h.square, lang);
            let tpl = if *was {
                p.pick(
                    35,
                    t(
                        lang,
                        &["This leaves {p} hanging", "This doesn't deal with the threat to {p}", "{P} is still en prise after this"],
                        &["Esta jugada deja {p} colgad{p_o}", "Esta jugada no resuelve la amenaza sobre {p}", "{P} sigue colgad{p_o} tras esta jugada"],
                        &["Este lance deixa {p} pendurad{p_o}", "Este lance não resolve a ameaça sobre {p}", "{P} continua pendurad{p_o} depois deste lance"],
                        &["Ce coup laisse {p} en prise", "Ce coup ne règle pas la menace sur {p}", "{P} reste en prise après ce coup"],
                        &["Dieser Zug lässt {p_acc} hängen", "Dieser Zug ändert nichts an der Drohung gegen {p_acc}", "{P} hängt nach diesem Zug immer noch"],
                    ),
                )
            } else if h.undefended {
                p.pick(
                    36,
                    t(
                        lang,
                        &["This hangs {p}", "This leaves {p} undefended", "This drops {p}"],
                        &["Esta jugada deja colgad{p_o} {p}", "Esta jugada deja {p} sin defensa", "Con esta jugada, {p} queda colgad{p_o}"],
                        &["Este lance entrega {p}", "Este lance deixa {p} sem defesa", "Com este lance, {p} fica pendurad{p_o}"],
                        &["Ce coup met {p} en prise", "Ce coup laisse {p} sans défense", "Avec ce coup, {p} est en prise"],
                        &["Dieser Zug stellt {p_acc} ein", "Dieser Zug lässt {p_acc} ungedeckt", "Nach diesem Zug hängt {p}"],
                    ),
                )
            } else {
                p.pick(
                    37,
                    t(
                        lang,
                        &["This puts {p} where it can be won", "This leaves {p} under-protected"],
                        &["Esta jugada pone {p} donde se puede perder", "Esta jugada deja {p} mal defendid{p_o}"],
                        &["Este lance coloca {p} onde pode ser perdid{p_o}", "Este lance deixa {p} mal defendid{p_o}"],
                        &["Ce coup place {p} là où {p_er} peut être perdu{p_o}", "Ce coup laisse {p} mal défendu{p_o}"],
                        &["Dieser Zug stellt {p_acc} auf ein Feld, wo {p_er} verloren gehen kann", "Nach diesem Zug ist {p} zu schwach gedeckt"],
                    ),
                )
            };
            let mut sent = fill(tpl, &v);
            match cap {
                Some(c) => {
                    let tail = l5(lang, [" — {c} wins it.", ": {c} {p_lo} gana.", ": {c} {p_lo} captura.", " : {c} {p_lo} prend.", ": {c} schlägt {p_lo}."]);
                    sent.push_str(&fill(tail, &cat(v, kv(&[("c", c)]))));
                }
                None => sent.push('.'),
            }
            out.push(sent);
        }
    }
    if out.is_empty() && f.captured.is_some() && f.capture_net <= -90 {
        let tpl = p.pick(
            38,
            t(
                lang,
                &[
                    "This capture doesn't add up — after the recapture you lose material.",
                    "The trade here costs material once the opponent takes back.",
                ],
                &[
                    "Esta jugada no compensa: tras la recaptura pierdes material.",
                    "Este cambio cuesta material en cuanto el rival recaptura.",
                ],
                &[
                    "Este lance não compensa: depois da recaptura você perde material.",
                    "Esta troca custa material assim que o adversário recaptura.",
                ],
                &[
                    "Ce coup ne tient pas : après la reprise, tu perds du matériel.",
                    "Cet échange coûte du matériel dès que l'adversaire reprend.",
                ],
                &[
                    "Dieser Zug geht nicht auf: Nach dem Zurückschlagen verlierst du Material.",
                    "Dieser Tausch kostet Material, sobald der Gegner zurückschlägt.",
                ],
            ),
        );
        out.push(tpl.to_string());
    }
    if out.is_empty() && cat_ == Cat::Miss {
        if let Some(b) = best {
            if let Some(cap) = b.captured.filter(|_| b.wins_material()) {
                let tpl = l5(
                    lang,
                    [
                        "This misses a chance: {san} would have won {c_un}.",
                        "Esta jugada deja pasar una oportunidad: {san} habría ganado {c_un}.",
                        "Este lance deixa passar uma oportunidade: {san} teria ganhado {c_un}.",
                        "Ce coup laisse passer une occasion : {san} aurait gagné {c_un}.",
                        "Dieser Zug verpasst eine Chance: {san} hätte {c_un_acc} gewonnen.",
                    ],
                );
                out.push(fill(tpl, &cat(rv("c", cap, lang), kv(&[("san", &b.san)]))));
            } else if b.fork.len() >= 2 {
                let tpl = l5(
                    lang,
                    [
                        "This misses a fork with {san}.",
                        "Esta jugada deja pasar un ataque doble con {san}.",
                        "Este lance deixa passar um garfo com {san}.",
                        "Ce coup laisse passer une fourchette avec {san}.",
                        "Dieser Zug verpasst eine Gabel mit {san}.",
                    ],
                );
                out.push(fill(tpl, &kv(&[("san", &b.san)])));
            } else {
                let tpl = p.pick(
                    39,
                    t(
                        lang,
                        &["This misses a chance to punish the opponent's last move.", "The opponent slipped up, but this doesn't take advantage."],
                        &["Esta jugada no castiga la última jugada del rival.", "El rival se equivocó, pero esta jugada no lo aprovecha."],
                        &["Este lance não pune o último lance do adversário.", "O adversário errou, mas este lance não aproveita."],
                        &["Ce coup ne punit pas le dernier coup de l'adversaire.", "L'adversaire s'est trompé, mais ce coup n'en profite pas."],
                        &["Dieser Zug bestraft den letzten Zug des Gegners nicht.", "Der Gegner hat gepatzt, aber dieser Zug nutzt das nicht aus."],
                    ),
                );
                out.push(tpl.to_string());
            }
        }
    }
    if out.is_empty() && f.king_walk {
        out.push(
            l5(
                lang,
                [
                    "Moving the king this early gives up castling and leaves it exposed.",
                    "Mover el rey tan pronto renuncia al enroque y lo deja expuesto.",
                    "Mover o rei tão cedo abre mão do roque e o deixa exposto.",
                    "Déplacer le roi si tôt renonce au roque et le laisse exposé.",
                    "Den König so früh zu ziehen kostet die Rochade und lässt ihn ungeschützt.",
                ],
            )
            .to_string(),
        );
    }
    if out.is_empty() && f.weakens_king {
        out.push(
            l5(
                lang,
                [
                    "Pushing a pawn in front of the castled king weakens its shelter.",
                    "Avanzar un peón delante del rey enrocado debilita su refugio.",
                    "Avançar um peão na frente do rei rocado enfraquece seu abrigo.",
                    "Pousser un pion devant le roi roqué affaiblit son abri.",
                    "Ein Bauernzug vor dem rochierten König schwächt seine Deckung.",
                ],
            )
            .to_string(),
        );
    }
    out
}

/// The sentence subject that stands for "the move" in templates ("This" / "Esta jugada").
fn subject(lang: Lang) -> &'static str {
    l5(lang, ["This", "Esta jugada", "Este lance", "Ce coup", "Dieser Zug"])
}

fn eval_drop(ctx: &MoveContext, mover: Color) -> i32 {
    let before = pov_cp(ctx.eval_before, mover).clamp(-2000, 2000);
    let after = pov_cp(ctx.eval_after, mover).clamp(-2000, 2000);
    (before - after).max(0)
}

fn generic_bad(cat_: Cat, ctx: &MoveContext, mover: Color, p: &Picker, lang: Lang) -> String {
    let after = pov_cp(ctx.eval_after, mover);
    match cat_ {
        Cat::Blunder if after < -150 => p.pick(
            41,
            t(
                lang,
                &["This turns the game around for the opponent.", "This seriously damages the position."],
                &["Esta jugada le da la vuelta a la partida a favor del rival.", "Esta jugada daña seriamente la posición."],
                &["Este lance vira o jogo a favor do adversário.", "Este lance prejudica seriamente a posição."],
                &["Ce coup renverse la partie en faveur de l'adversaire.", "Ce coup abîme sérieusement la position."],
                &["Dieser Zug dreht die Partie zugunsten des Gegners.", "Dieser Zug schwächt die Stellung erheblich."],
            ),
        ),
        Cat::Blunder | Cat::Mistake => p.pick(
            42,
            t(
                lang,
                &["This gives away a big part of the advantage.", "This lets the opponent take over.", "This hands the opponent a strong game."],
                &["Esta jugada regala buena parte de la ventaja.", "Esta jugada permite que el rival tome el control.", "Esta jugada le pone la partida muy fácil al rival."],
                &["Este lance entrega boa parte da vantagem.", "Este lance permite que o adversário assuma o controle.", "Este lance deixa a partida bem fácil para o adversário."],
                &["Ce coup gâche une bonne partie de l'avantage.", "Ce coup permet à l'adversaire de prendre le dessus.", "Ce coup offre une partie facile à l'adversaire."],
                &["Dieser Zug verschenkt einen großen Teil des Vorteils.", "Dieser Zug lässt den Gegner die Kontrolle übernehmen.", "Dieser Zug macht dem Gegner das Leben leicht."],
            ),
        ),
        _ => p.pick(
            43,
            t(
                lang,
                &["This is a bit slow.", "This is playable, but there was something more active.", "A small slip — the position gets a little harder."],
                &["Esta jugada es un poco lenta.", "Se puede jugar, pero había algo más activo.", "Un pequeño desliz: la posición se complica un poco."],
                &["Este lance é um pouco lento.", "Dá para jogar, mas havia algo mais ativo.", "Um pequeno deslize: a posição fica um pouco mais difícil."],
                &["Ce coup est un peu lent.", "C'est jouable, mais il y avait plus actif.", "Une petite imprécision : la position devient un peu plus difficile."],
                &["Dieser Zug ist etwas langsam.", "Spielbar, aber es gab etwas Aktiveres.", "Ein kleiner Ausrutscher – die Stellung wird etwas schwieriger."],
            ),
        ),
    }
    .to_string()
}

fn fallback_text(ctx: &MoveContext, cat_: Cat, lang: Lang) -> String {
    let played = if ctx.played_san.is_empty() { ctx.played_uci.as_str() } else { ctx.played_san.as_str() };
    let best = if ctx.best_san.is_empty() { ctx.best_uci.as_str() } else { ctx.best_san.as_str() };
    let better = if !best.is_empty() && best != played {
        fill(l5(lang, [" {b} was better.", " {b} era mejor.", " {b} era melhor.", " {b} était meilleur.", " {b} war besser."]), &kv(&[("b", best)]))
    } else {
        String::new()
    };
    let texts: [&str; 5] = match cat_ {
        Cat::Brilliant => [
            "A brilliant, hard-to-find move!",
            "¡Una jugada brillante y difícil de encontrar!",
            "Um lance brilhante e difícil de encontrar!",
            "Un coup brillant et difficile à trouver !",
            "Ein brillanter, schwer zu findender Zug!",
        ],
        Cat::Great => [
            "A great move — the key idea in this position.",
            "Una gran jugada: la idea clave de esta posición.",
            "Um ótimo lance: a ideia-chave desta posição.",
            "Un très bon coup : l'idée clé de cette position.",
            "Ein starker Zug – die Kernidee dieser Stellung.",
        ],
        Cat::Best => ["That's the best move here.", "Esa es la mejor jugada aquí.", "Esse é o melhor lance aqui.", "C'est le meilleur coup ici.", "Das ist hier der beste Zug."],
        Cat::Good => ["A solid move.", "Una jugada sólida.", "Um lance sólido.", "Un coup solide.", "Ein solider Zug."],
        Cat::Book => [
            "A well-known opening move.",
            "Una jugada de teoría muy conocida.",
            "Um lance de teoria bem conhecido.",
            "Un coup théorique bien connu.",
            "Ein bekannter Theoriezug.",
        ],
        Cat::Forced => [
            "This was the only reasonable move.",
            "Era la única jugada razonable.",
            "Era o único lance razoável.",
            "C'était le seul coup raisonnable.",
            "Das war der einzige vernünftige Zug.",
        ],
        Cat::Inaccuracy => ["A little imprecise.", "Un poco imprecisa.", "Um pouco impreciso.", "Un peu imprécis.", "Etwas ungenau."],
        Cat::Mistake => ["This is a mistake.", "Esto es un error.", "Isto é um erro.", "C'est une erreur.", "Das ist ein Fehler."],
        Cat::Miss => [
            "This misses a chance.",
            "Esto deja pasar una oportunidad.",
            "Isto deixa passar uma oportunidade.",
            "Cela laisse passer une occasion.",
            "Damit wird eine Chance verpasst.",
        ],
        Cat::Blunder => ["This is a serious error.", "Esto es un error grave.", "Isto é um erro grave.", "C'est une gaffe.", "Das ist ein grober Fehler."],
    };
    let text = l5(lang, texts);
    match cat_ {
        Cat::Good | Cat::Inaccuracy | Cat::Mistake | Cat::Miss | Cat::Blunder => format!("{text}{better}"),
        _ => text.to_string(),
    }
}

fn sentence(clause: &str) -> String {
    let c = clause.trim_end_matches('.');
    format!("{}.", c)
}

pub fn explain_move(ctx: &MoveContext, lang: Lang) -> String {
    let Ok(pos) = gm_engine::parse_fen(&ctx.fen_before) else {
        return fallback_text(ctx, category(&ctx.classification, 0), lang);
    };
    let played = tactics::parse_any_move(&pos, &ctx.played_uci).or_else(|| tactics::parse_any_move(&pos, &ctx.played_san));
    let Some(played) = played else {
        return fallback_text(ctx, category(&ctx.classification, 0), lang);
    };
    let mover = pos.turn();
    let cat_ = category(&ctx.classification, eval_drop(ctx, mover));
    // The picker seed does not depend on the language, so the same variant index is used in
    // every language (stable explanations when switching back and forth).
    let p = Picker::new(&[&ctx.fen_before, &ctx.played_uci, &ctx.played_san]);
    let f = tactics::move_facts(&pos, &played);

    let best_move = tactics::parse_any_move(&pos, &ctx.best_uci).or_else(|| tactics::parse_any_move(&pos, &ctx.best_san));
    let best_is_played = best_move.as_ref().map(|b| *b == played).unwrap_or(true);
    let best_facts = match &best_move {
        Some(b) if !best_is_played => Some(tactics::move_facts(&pos, b)),
        _ => None,
    };

    explain_with(ctx, &pos, cat_, &f, best_facts.as_ref(), &p, lang)
}

/// Engine-free verdict on a move, used by the chat fallback.
pub(crate) struct Comment {
    /// The move has a concrete problem (hangs material, allows mate...).
    pub bad: bool,
    /// Nothing tactical to say about it.
    pub quiet: bool,
    pub text: String,
}

pub(crate) fn plain_move_comment(pos: &Chess, m: &shakmaty::Move, lang: Lang) -> Comment {
    let f = tactics::move_facts(pos, m);
    let fen = gm_engine::to_fen(pos);
    let p = Picker::new(&[&fen, &f.san]);
    let san = kv(&[("san", &f.san)]);
    if f.is_mate {
        let text = fill(
            l5(lang, ["**{san}** is checkmate!", "¡**{san}** es jaque mate!", "**{san}** é xeque-mate!", "**{san}** est échec et mat !", "**{san}** ist Schachmatt!"]),
            &san,
        );
        return Comment { bad: false, quiet: false, text };
    }
    let ctx = MoveContext { fen_before: fen.clone(), ..Default::default() };
    let probs = problems(&ctx, &f, None, Cat::Mistake, &p, lang);
    if let Some(first) = probs.into_iter().next() {
        return Comment { bad: true, quiet: false, text: first.replacen(subject(lang), &format!("**{}**", f.san), 1) };
    }
    let reasons = positive_reasons(&f, &p, lang);
    match reasons.first() {
        Some(r) => Comment { bad: false, quiet: false, text: format!("**{}** {r}.", f.san) },
        None => Comment {
            bad: false,
            quiet: true,
            text: fill(
                l5(
                    lang,
                    [
                        "**{san}** is a quiet move — it doesn't create or allow any immediate tactics.",
                        "**{san}** es una jugada tranquila: no crea ni permite ninguna táctica inmediata.",
                        "**{san}** é um lance tranquilo: não cria nem permite nenhuma tática imediata.",
                        "**{san}** est un coup calme : il ne crée ni ne permet aucune tactique immédiate.",
                        "**{san}** ist ein ruhiger Zug – er schafft keine unmittelbare Taktik und lässt auch keine zu.",
                    ],
                ),
                &san,
            ),
        },
    }
}

fn better_clause(b: &MoveFacts, p: &Picker, lang: Lang) -> String {
    let reasons = positive_reasons(b, p, lang);
    let opener = p.pick(
        51,
        t(
            lang,
            &["Better was", "Stronger was", "The engine prefers"],
            &["Era mejor", "Más fuerte era", "El motor prefiere"],
            &["Era melhor", "Mais forte era", "O motor prefere"],
            &["Il fallait jouer", "Mieux valait", "Le moteur préfère"],
            &["Besser war", "Stärker war", "Die Engine bevorzugt"],
        ),
    );
    let v = kv(&[("o", opener), ("san", &b.san)]);
    if b.is_mate {
        let tpl = l5(
            lang,
            ["{o} {san}, which is checkmate!", "{o} {san}, ¡que es jaque mate!", "{o} {san}, que é xeque-mate!", "{o} {san}, qui fait échec et mat !", "{o} {san} – das ist Schachmatt!"],
        );
        return fill(tpl, &v);
    }
    match reasons.first() {
        Some(r) => {
            let complex = match lang {
                Lang::En => r.contains("which") || r.contains(','),
                Lang::Es | Lang::Pt => r.contains(" que ") || r.contains(',') || r.contains(':'),
                Lang::Fr => r.contains(" qui ") || r.contains(" que ") || r.contains(',') || r.contains(':'),
                // German relative clauses need verb-final order, so the reason always gets
                // its own main clause.
                Lang::De => true,
            };
            let tpl = if complex {
                l5(lang, ["{o} {san} — it {r}.", "{o} {san}: {r}.", "{o} {san}: {r}.", "{o} {san} : il {r}.", "{o} {san} – dieser Zug {r}."])
            } else {
                l5(lang, ["{o} {san}, which {r}.", "{o} {san}, que {r}.", "{o} {san}, que {r}.", "{o} {san}, qui {r}.", "{o} {san} – dieser Zug {r}."])
            };
            fill(tpl, &cat(v, kv(&[("r", r)])))
        }
        None => fill("{o} {san}.", &v),
    }
}

fn explain_with(ctx: &MoveContext, pos: &Chess, cat_: Cat, f: &MoveFacts, best: Option<&MoveFacts>, p: &Picker, lang: Lang) -> String {
    let mover = f.mover;
    if f.is_mate {
        return p
            .pick(
                1,
                t(
                    lang,
                    &["Checkmate! Beautifully finished.", "Checkmate — game over. Well played!", "That's checkmate! Nicely done."],
                    &["¡Jaque mate! Un final precioso.", "Jaque mate: se acabó la partida. ¡Bien jugado!", "¡Eso es jaque mate! Muy bien hecho."],
                    &["Xeque-mate! Uma finalização linda.", "Xeque-mate: fim de jogo. Muito bem jogado!", "Isso é xeque-mate! Muito bem feito."],
                    &["Échec et mat ! Superbe finition.", "Échec et mat : la partie est terminée. Bien joué !", "C'est échec et mat ! Bravo."],
                    &["Schachmatt! Wunderschön zu Ende gespielt.", "Schachmatt – die Partie ist vorbei. Gut gespielt!", "Das ist Schachmatt! Sehr gut gemacht."],
                ),
            )
            .to_string();
    }

    if cat_.is_bad() {
        let mut parts = problems(ctx, f, best, cat_, p, lang);
        if parts.is_empty() {
            parts.push(generic_bad(cat_, ctx, mover, p, lang));
        }
        parts.truncate(2);
        if let Some(b) = best.filter(|b| !parts.iter().any(|t| t.contains(&b.san))) {
            parts.push(better_clause(b, p, lang));
        }
        return parts.join(" ");
    }

    let reasons = positive_reasons(f, p, lang);
    let and = l5(lang, [" and ", " y ", " e ", " et ", " und "]);
    let reason_sentence = |lead: &str| -> Option<String> {
        let mut rs = reasons.iter().take(2);
        let first = rs.next()?;
        let mut s = format!("{lead} {first}");
        let simple = |c: &str| !c.contains(',') && !c.contains('—') && !c.contains('–') && !c.contains(':') && !c.contains(and);
        if let Some(second) = rs.next().filter(|sec| simple(first) && simple(sec)) {
            s.push_str(and);
            s.push_str(second);
        }
        Some(sentence(&s))
    };
    let this = subject(lang);
    let it = l5(lang, ["It", "Esta jugada", "Este lance", "Il", "Er"]);

    match cat_ {
        Cat::Forced => {
            if pos.legal_moves().len() == 1 {
                l5(
                    lang,
                    ["This was the only legal move.", "Era la única jugada legal.", "Era o único lance legal.", "C'était le seul coup légal.", "Das war der einzige legale Zug."],
                )
                .to_string()
            } else {
                p.pick(
                    2,
                    t(
                        lang,
                        &["This was the only move that holds things together.", "Forced — everything else loses."],
                        &["Era la única jugada que lo sostenía todo.", "Forzada: todo lo demás pierde."],
                        &["Era o único lance que segurava tudo.", "Forçado: todo o resto perde."],
                        &["C'était le seul coup qui tenait tout.", "Forcé : tout le reste perd."],
                        &["Das war der einzige Zug, der alles zusammenhält.", "Erzwungen – alles andere verliert."],
                    ),
                )
                .to_string()
            }
        }
        Cat::Book => {
            let lead = p.pick(
                3,
                t(
                    lang,
                    &["A well-known opening move.", "Straight out of opening theory.", "A book move — this is how the masters play it."],
                    &["Una jugada de apertura muy conocida.", "Directamente de la teoría de aperturas.", "Una jugada de teoría: así la juegan los maestros."],
                    &["Um lance de abertura bem conhecido.", "Direto da teoria de aberturas.", "Um lance de teoria: é assim que os mestres jogam."],
                    &["Un coup d'ouverture bien connu.", "Tout droit sorti de la théorie des ouvertures.", "Un coup théorique : c'est ainsi que jouent les maîtres."],
                    &["Ein bekannter Eröffnungszug.", "Direkt aus der Eröffnungstheorie.", "Ein Theoriezug – so spielen ihn die Meister."],
                ),
            );
            match reason_sentence(it) {
                Some(r) => format!("{lead} {r}"),
                None => lead.to_string(),
            }
        }
        Cat::Brilliant => {
            let sac = f.hangs.first().map(|(h, _, _)| h.role).or((f.capture_net < 0).then_some(f.role));
            let lead = match sac {
                Some(r) => {
                    let excl = p.pick(
                        4,
                        t(
                            lang,
                            &["Brilliant!", "Wow — brilliant!", "A brilliant sacrifice!"],
                            &["¡Brillante!", "¡Guau, brillante!", "¡Un sacrificio brillante!"],
                            &["Brilhante!", "Uau, brilhante!", "Um sacrifício brilhante!"],
                            &["Brillant !", "Waouh, brillant !", "Un sacrifice brillant !"],
                            &["Brillant!", "Wow – brillant!", "Ein brillantes Opfer!"],
                        ),
                    );
                    let tail = fill(
                        l5(
                            lang,
                            [
                                "Giving up {p} is the key idea.",
                                "Entregar {p} es la idea clave.",
                                "Entregar {p} é a ideia-chave.",
                                "Sacrifier {p} est l'idée clé.",
                                "Das Opfer {p_del} ist die Kernidee.",
                            ],
                        ),
                        &rv("p", r, lang),
                    );
                    format!("{excl} {tail}")
                }
                None => p
                    .pick(
                        5,
                        t(
                            lang,
                            &["Brilliant!", "A brilliant find!"],
                            &["¡Brillante!", "¡Un hallazgo brillante!"],
                            &["Brilhante!", "Um achado brilhante!"],
                            &["Brillant !", "Une trouvaille brillante !"],
                            &["Brillant!", "Ein brillanter Fund!"],
                        ),
                    )
                    .to_string(),
            };
            match reason_sentence(this) {
                Some(r) => format!("{lead} {r}"),
                None => lead,
            }
        }
        Cat::Great => {
            let lead = p.pick(
                6,
                t(
                    lang,
                    &["Great find!", "Great move!", "Excellent spot!"],
                    &["¡Gran hallazgo!", "¡Gran jugada!", "¡Excelente visión!"],
                    &["Ótimo achado!", "Ótimo lance!", "Excelente visão!"],
                    &["Belle trouvaille !", "Très bon coup !", "Excellent coup d'œil !"],
                    &["Starker Fund!", "Starker Zug!", "Hervorragend gesehen!"],
                ),
            );
            match reason_sentence(this) {
                Some(r) => format!("{lead} {r}"),
                None => {
                    let tail = l5(
                        lang,
                        [
                            "It's the only move that keeps the advantage.",
                            "Es la única jugada que mantiene la ventaja.",
                            "É o único lance que mantém a vantagem.",
                            "C'est le seul coup qui garde l'avantage.",
                            "Es ist der einzige Zug, der den Vorteil hält.",
                        ],
                    );
                    format!("{lead} {tail}")
                }
            }
        }
        Cat::Best => {
            let lead = p.pick(
                7,
                t(
                    lang,
                    &["Nice!", "Well played!", "Spot on!", "Exactly right!"],
                    &["¡Bien!", "¡Bien jugado!", "¡Exacto!", "¡Justo eso!"],
                    &["Boa!", "Muito bem jogado!", "Exato!", "Isso mesmo!"],
                    &["Bien !", "Bien joué !", "Exactement !", "C'est tout à fait ça !"],
                    &["Schön!", "Gut gespielt!", "Genau!", "Ganz genau richtig!"],
                ),
            );
            match reason_sentence(this) {
                Some(r) => format!("{lead} {r}"),
                None => {
                    let ev = pov_cp(ctx.eval_after, mover);
                    let tail = if ev >= 300 {
                        [
                            "This keeps a winning position under control.",
                            "Así mantienes bajo control una posición ganada.",
                            "Assim você mantém sob controle uma posição ganha.",
                            "Tu gardes ainsi le contrôle d'une position gagnante.",
                            "So behältst du eine gewonnene Stellung unter Kontrolle.",
                        ]
                    } else if ev <= -300 {
                        [
                            "It's the most stubborn defense available.",
                            "Es la defensa más tenaz posible.",
                            "É a defesa mais teimosa possível.",
                            "C'est la défense la plus tenace possible.",
                            "Das ist die zäheste Verteidigung.",
                        ]
                    } else {
                        [
                            "It's the engine's top choice here.",
                            "Es la primera opción del motor aquí.",
                            "É a primeira opção do motor aqui.",
                            "C'est le premier choix du moteur ici.",
                            "Das ist hier die erste Wahl der Engine.",
                        ]
                    };
                    format!("{lead} {}", l5(lang, tail))
                }
            }
        }
        _ => {
            // Good / excellent.
            let lead = p.pick(
                8,
                t(
                    lang,
                    &["Good move.", "Solid.", "A good, healthy move."],
                    &["Buena jugada.", "Sólida.", "Una jugada buena y sana."],
                    &["Bom lance.", "Sólido.", "Um lance bom e saudável."],
                    &["Bon coup.", "Solide.", "Un bon coup, sain."],
                    &["Guter Zug.", "Solide.", "Ein guter, gesunder Zug."],
                ),
            );
            let mut s = match reason_sentence(this) {
                Some(r) => format!("{lead} {r}"),
                None => lead.to_string(),
            };
            if let Some(b) = best {
                s.push(' ');
                let tpl = p.pick(
                    9,
                    t(
                        lang,
                        &["{b} was slightly more precise.", "{b} was a touch stronger.", "The engine slightly prefers {b}."],
                        &["{b} era un poco más precisa.", "{b} era algo más fuerte.", "El motor prefiere ligeramente {b}."],
                        &["{b} era um pouco mais preciso.", "{b} era um pouco mais forte.", "O motor prefere levemente {b}."],
                        &["{b} était un peu plus précis.", "{b} était légèrement plus fort.", "Le moteur préfère légèrement {b}."],
                        &["{b} war etwas genauer.", "{b} war einen Tick stärker.", "Die Engine bevorzugt leicht {b}."],
                    ),
                );
                s.push_str(&fill(tpl, &kv(&[("b", &b.san)])));
            }
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(fen: &str, played: &str, best: &str, cls: &str, before: Score, after: Score) -> MoveContext {
        MoveContext {
            fen_before: fen.into(),
            played_uci: played.into(),
            best_uci: best.into(),
            eval_before: before,
            eval_after: after,
            classification: cls.into(),
            ..Default::default()
        }
    }

    #[test]
    fn hanging_piece_blunder() {
        // Two Knights: Bc4-a6?? hangs the bishop to bxa6.
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4";
        let text = explain_move(&ctx(fen, "c4a6", "e1g1", "blunder", Score::Cp(30), Score::Cp(-300)), Lang::En);
        assert!(text.contains("bishop on a6"), "{text}");
        assert!(text.contains("bxa6"), "{text}");
        assert!(text.contains("O-O"), "{text}");
    }

    #[test]
    fn losing_capture_beats_unrelated_hanging_pawn() {
        // 1.e4 e5 2.Qh5 Nc6 3.Qxf7+?? Kxf7: about the lost queen, not the e4 pawn.
        let fen = "r1bqkbnr/pppp1ppp/2n5/4p2Q/4P3/8/PPPP1PPP/RNB1KBNR w KQkq - 2 3";
        let text = explain_move(&ctx(fen, "h5f7", "f1c4", "blunder", Score::Cp(-20), Score::Cp(-850)), Lang::En);
        assert!(!text.contains("e4"), "{text}");
        assert!(text.contains("recapture"), "{text}");
    }

    #[test]
    fn fork_praised() {
        // White knight jumps to c7 forking king e8 and rook a8.
        let fen = "r3k3/8/8/1N6/8/8/8/4K3 w - - 0 1";
        let text = explain_move(&ctx(fen, "b5c7", "b5c7", "best", Score::Cp(100), Score::Cp(500)), Lang::En);
        assert!(text.to_lowercase().contains("fork"), "{text}");
        assert!(text.contains("rook on a8"), "{text}");
    }

    #[test]
    fn mate_delivered_and_missed() {
        let fen = "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1";
        let text = explain_move(&ctx(fen, "a1a8", "a1a8", "best", Score::Mate(1), Score::Mate(0)), Lang::En);
        assert!(text.to_lowercase().contains("checkmate"), "{text}");
        let text = explain_move(&ctx(fen, "a1a7", "a1a8", "miss", Score::Mate(1), Score::Cp(500)), Lang::En);
        assert!(text.contains("Ra8#"), "{text}");
    }

    #[test]
    fn allows_back_rank_mate() {
        // Black to move; white threatens Re8# if black ignores it.
        let fen = "6k1/5ppp/8/8/8/8/5PPP/4R1K1 b - - 0 1";
        let text = explain_move(&ctx(fen, "a7a6", "h7h6", "blunder", Score::Cp(0), Score::Mate(1)), Lang::En);
        // a7a6 is illegal here (no pawn) -> fallback still friendly.
        assert!(!text.is_empty());
        let fen = "r5k1/5ppp/8/8/8/8/5PPP/4R1K1 b - - 0 1";
        let text = explain_move(&ctx(fen, "a8a7", "h7h6", "blunder", Score::Cp(0), Score::Mate(1)), Lang::En);
        assert!(text.contains("Re8#"), "{text}");
        assert!(text.contains("back-rank"), "{text}");
    }

    #[test]
    fn development_and_castling() {
        let fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
        let text = explain_move(&ctx(fen, "g1f3", "g1f3", "best", Score::Cp(30), Score::Cp(30)), Lang::En);
        assert!(text.contains("knight"), "{text}");
        let fen = "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4";
        let text = explain_move(&ctx(fen, "e1g1", "e1g1", "book", Score::Cp(30), Score::Cp(30)), Lang::En);
        assert!(text.to_lowercase().contains("king"), "{text}");
    }

    #[test]
    fn garbage_input_does_not_panic() {
        let text = explain_move(&ctx("not a fen", "zz", "", "", Score::Cp(0), Score::Mate(-3)), Lang::En);
        assert!(!text.is_empty());
        let text = explain_move(&MoveContext::default(), Lang::En);
        assert!(!text.is_empty());
    }

    // ---- Spanish ------------------------------------------------------------------------------

    fn no_english(text: &str) {
        for w in [" the ", "This ", " was ", " on a", " with "] {
            assert!(!text.contains(w), "English leaked into Spanish: {text}");
        }
    }

    #[test]
    fn spanish_hanging_piece() {
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4";
        let text = explain_move(&ctx(fen, "c4a6", "e1g1", "blunder", Score::Cp(30), Score::Cp(-300)), Lang::Es);
        assert!(text.contains("alfil en a6"), "{text}");
        assert!(text.contains("bxa6"), "{text}");
        assert!(text.contains("O-O"), "{text}");
        // Masculine agreement for the bishop.
        assert!(!text.contains("colgada") && !text.contains("indefensa") && !text.contains("defendida"), "{text}");
        assert!(text.contains("lo gana") || !text.contains("gana"), "{text}");
        no_english(&text);
        // Every phrasing variant agrees with the bishop's gender.
        for salt in ["", "x", "y", "z", "w"] {
            let mut c = ctx(fen, "c4a6", "e1g1", "blunder", Score::Cp(30), Score::Cp(-300));
            c.played_san = salt.into();
            let t = explain_move(&c, Lang::Es);
            assert!(!t.contains("colgada") && !t.contains("la gana"), "{t}");
        }
    }

    #[test]
    fn spanish_hanging_queen_is_feminine() {
        // 1.e4 e5 2.Qg4?? hangs the queen to ...Bxg4? No: d7 pawn opens... use a simple position:
        // White queen steps to d5 where the black knight on f6 takes it for free.
        let fen = "rnbqkb1r/pppp1ppp/5n2/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 2 3";
        let text = explain_move(&ctx(fen, "d1h5", "b1c3", "blunder", Score::Cp(30), Score::Cp(-800)), Lang::Es);
        assert!(text.contains("dama en h5"), "{text}");
        assert!(text.contains("Nxh5"), "{text}");
        assert!(text.contains("la gana") || text.contains("colgada") || text.contains("defendida") || text.contains("sin defensa") || text.contains("perder") || text.contains("amenaza"), "{text}");
        assert!(!text.contains("colgado") && !text.contains("lo gana"), "{text}");
        no_english(&text);
    }

    #[test]
    fn spanish_fork() {
        let fen = "r3k3/8/8/1N6/8/8/8/4K3 w - - 0 1";
        let text = explain_move(&ctx(fen, "b5c7", "b5c7", "best", Score::Cp(100), Score::Cp(500)), Lang::Es);
        assert!(text.contains("ataque doble"), "{text}");
        assert!(text.contains("torre en a8"), "{text}");
        assert!(text.contains("rey"), "{text}");
        assert!(!text.contains("a el ") && !text.contains("de el "), "contractions: {text}");
        no_english(&text);
    }

    #[test]
    fn spanish_mates() {
        let fen = "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1";
        let text = explain_move(&ctx(fen, "a1a8", "a1a8", "best", Score::Mate(1), Score::Mate(0)), Lang::Es);
        assert!(text.to_lowercase().contains("jaque mate"), "{text}");
        let text = explain_move(&ctx(fen, "a1a7", "a1a8", "miss", Score::Mate(1), Score::Cp(500)), Lang::Es);
        assert!(text.contains("Ra8#") && text.contains("mate en una"), "{text}");
        let fen = "r5k1/5ppp/8/8/8/8/5PPP/4R1K1 b - - 0 1";
        let text = explain_move(&ctx(fen, "a8a7", "h7h6", "blunder", Score::Cp(0), Score::Mate(1)), Lang::Es);
        assert!(text.contains("Re8#") && text.contains("mate del pasillo"), "{text}");
        no_english(&text);
    }

    #[test]
    fn spanish_development_castling_and_fallbacks() {
        let fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
        let text = explain_move(&ctx(fen, "g1f3", "g1f3", "best", Score::Cp(30), Score::Cp(30)), Lang::Es);
        assert!(text.contains("caballo") || text.contains("pieza"), "{text}");
        no_english(&text);
        let fen = "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4";
        let text = explain_move(&ctx(fen, "e1g1", "e1g1", "book", Score::Cp(30), Score::Cp(30)), Lang::Es);
        assert!(text.contains("rey") || text.contains("enroca"), "{text}");
        let text = explain_move(&ctx("not a fen", "zz", "e2e4", "mistake", Score::Cp(0), Score::Cp(0)), Lang::Es);
        assert_eq!(text, "Esto es un error. e2e4 era mejor.");
    }

    #[test]
    fn spanish_plain_comment_substitutes_san() {
        let pos = gm_engine::parse_fen("r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4").expect("fen");
        let m = tactics::parse_any_move(&pos, "c4a6").expect("move");
        let c = plain_move_comment(&pos, &m, Lang::Es);
        assert!(c.bad, "{}", c.text);
        assert!(c.text.contains("**Ba6**"), "{}", c.text);
    }

    // ---- Portuguese, French, German -------------------------------------------------------------

    const NEW_LANGS: [Lang; 3] = [Lang::Pt, Lang::Fr, Lang::De];

    fn clean(text: &str, lang: Lang) {
        assert!(!text.trim().is_empty(), "{lang}: empty");
        assert!(!text.contains('{') && !text.contains('}'), "{lang}: unfilled placeholder: {text}");
        for w in [" the ", "This ", " which ", " with ", " was ", " it "] {
            assert!(!text.contains(w), "{lang}: English leaked: {text}");
        }
    }

    #[test]
    fn new_languages_hanging_bishop() {
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4";
        for (lang, phrase, wrong) in [
            (Lang::Pt, "bispo em a6", &["pendurada", "defendida", "perdida", "a captura"][..]),
            (Lang::Fr, "fou en a6", &["défendue", "perdue", "la prend"][..]),
            (Lang::De, "Läufer auf a6", &["die Läufer", "schlägt sie", "der Läufer auf a6 hängen"][..]),
        ] {
            for salt in ["", "x", "y", "z", "w"] {
                let mut c = ctx(fen, "c4a6", "e1g1", "blunder", Score::Cp(30), Score::Cp(-300));
                c.played_san = salt.into();
                let text = explain_move(&c, lang);
                assert!(text.contains(phrase), "{lang}: {text}");
                assert!(text.contains("bxa6") && text.contains("O-O"), "{lang}: {text}");
                for w in wrong {
                    assert!(!text.contains(w), "{lang}: agreement ({w}): {text}");
                }
                clean(&text, lang);
            }
        }
    }

    #[test]
    fn new_languages_hanging_queen_is_feminine() {
        let fen = "rnbqkb1r/pppp1ppp/5n2/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 2 3";
        for (lang, phrase, wrong) in [
            (Lang::Pt, "dama em h5", &["pendurado", "defendido", "perdido", "o captura", "o dama"][..]),
            (Lang::Fr, "dame en h5", &["le prend", "le dame", "défendu ", "défendu."][..]),
            (Lang::De, "Dame auf h5", &["schlägt ihn", "der Dame auf h5 hängen", "den Dame"][..]),
        ] {
            let text = explain_move(&ctx(fen, "d1h5", "b1c3", "blunder", Score::Cp(30), Score::Cp(-800)), lang);
            assert!(text.contains(phrase) && text.contains("Nxh5"), "{lang}: {text}");
            for w in wrong {
                assert!(!text.contains(w), "{lang}: agreement ({w}): {text}");
            }
            clean(&text, lang);
        }
    }

    #[test]
    fn new_languages_fork() {
        let fen = "r3k3/8/8/1N6/8/8/8/4K3 w - - 0 1";
        for (lang, fork, rook, king) in [
            (Lang::Pt, "garfo", "torre em a8", "rei"),
            (Lang::Fr, "fourchette", "tour en a8", "roi"),
            (Lang::De, "Gabel", "Turm auf a8", "König"),
        ] {
            let text = explain_move(&ctx(fen, "b5c7", "b5c7", "best", Score::Cp(100), Score::Cp(500)), lang);
            assert!(text.contains(fork) && text.contains(rook) && text.contains(king), "{lang}: {text}");
            assert!(![" a o ", " de o ", " em o ", " à le ", " de le "].iter().any(|c| text.contains(c)), "contractions: {text}");
            clean(&text, lang);
        }
    }

    #[test]
    fn new_languages_mates() {
        let fen = "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1";
        let br = "r5k1/5ppp/8/8/8/8/5PPP/4R1K1 b - - 0 1";
        for (lang, mate, in_one, back_rank) in [
            (Lang::Pt, "xeque-mate", "mate em um", "última fileira"),
            (Lang::Fr, "échec et mat", "mat en un coup", "mat du couloir"),
            (Lang::De, "schachmatt", "Matt in einem Zug", "Grundreihenmatt"),
        ] {
            let text = explain_move(&ctx(fen, "a1a8", "a1a8", "best", Score::Mate(1), Score::Mate(0)), lang);
            assert!(text.to_lowercase().contains(mate), "{lang}: {text}");
            clean(&text, lang);
            let text = explain_move(&ctx(fen, "a1a7", "a1a8", "miss", Score::Mate(1), Score::Cp(500)), lang);
            assert!(text.contains("Ra8#") && text.contains(in_one), "{lang}: {text}");
            clean(&text, lang);
            let text = explain_move(&ctx(br, "a8a7", "h7h6", "blunder", Score::Cp(0), Score::Mate(1)), lang);
            assert!(text.contains("Re8#") && text.contains(back_rank), "{lang}: {text}");
            clean(&text, lang);
        }
    }

    #[test]
    fn new_languages_development_castling_and_fallbacks() {
        let dev = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
        let castle = "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4";
        for (lang, knight, king, fallback) in [
            (Lang::Pt, ["cavalo", "peça"], ["rei", "roque"], "Isto é um erro. e2e4 era melhor."),
            (Lang::Fr, ["cavalier", "pièce"], ["roi", "roque"], "C'est une erreur. e2e4 était meilleur."),
            (Lang::De, ["Springer", "Figur"], ["König", "Rochade"], "Das ist ein Fehler. e2e4 war besser."),
        ] {
            let text = explain_move(&ctx(dev, "g1f3", "g1f3", "best", Score::Cp(30), Score::Cp(30)), lang);
            assert!(knight.iter().any(|w| text.contains(w)), "{lang}: {text}");
            clean(&text, lang);
            let text = explain_move(&ctx(castle, "e1g1", "e1g1", "book", Score::Cp(30), Score::Cp(30)), lang);
            assert!(king.iter().any(|w| text.contains(w)), "{lang}: {text}");
            clean(&text, lang);
            let text = explain_move(&ctx("not a fen", "zz", "e2e4", "mistake", Score::Cp(0), Score::Cp(0)), lang);
            assert_eq!(text, fallback);
        }
    }

    #[test]
    fn new_languages_plain_comment_substitutes_san() {
        let pos = gm_engine::parse_fen("r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4").expect("fen");
        let m = tactics::parse_any_move(&pos, "c4a6").expect("move");
        for lang in NEW_LANGS {
            let c = plain_move_comment(&pos, &m, lang);
            assert!(c.bad, "{}", c.text);
            assert!(c.text.contains("**Ba6**"), "{lang}: {}", c.text);
            clean(&c.text, lang);
        }
        let quiet = tactics::parse_any_move(&pos, "a2a3").expect("move");
        for lang in NEW_LANGS {
            let c = plain_move_comment(&pos, &quiet, lang);
            assert!(c.text.contains("**a3**"), "{lang}: {}", c.text);
            clean(&c.text, lang);
        }
    }

    /// Every classification, in every language, on positions with and without tactics,
    /// produces non-empty, fully filled text (and the fallback path too).
    #[test]
    fn never_empty_in_any_language() {
        let cases = [
            ("r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4", "c4a6", "e1g1"),
            ("r3k3/8/8/1N6/8/8/8/4K3 w - - 0 1", "b5c7", "b5c7"),
            ("rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2", "g1f3", "b1c3"),
            ("6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1", "a1a7", "a1a8"),
            ("not a fen", "zz", "e2e4"),
        ];
        let classes = ["brilliant", "great", "best", "excellent", "good", "book", "forced", "inaccuracy", "mistake", "miss", "blunder", ""];
        for lang in Lang::ALL {
            for (fen, played, best) in cases {
                for cls in classes {
                    let text = explain_move(&ctx(fen, played, best, cls, Score::Cp(50), Score::Cp(-200)), lang);
                    assert!(!text.trim().is_empty(), "{lang} {cls} {played}");
                    assert!(!text.contains('{') && !text.contains('}'), "{lang} {cls}: {text}");
                }
            }
            assert!(!subject(lang).is_empty());
        }
    }
}
