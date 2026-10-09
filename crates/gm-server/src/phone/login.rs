//! The small, self-contained sign-in page other devices see before they enter the PIN.
//! No scripts, no external files (everything else is behind the gate), localized server-side.

use gm_content::Lang;

use super::tr;

/// Errors shown on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginError {
    Wrong,
    /// Locked for this many seconds.
    Locked(u64),
    Origin,
}

/// HTML-escape text for element content and attribute values.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Percent-encode everything but unreserved characters and `/`.
pub fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decode `application/x-www-form-urlencoded` values (`+` is a space).
pub fn decode_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a=1&b=2` → value of `key`.
pub fn form_value(body: &str, key: &str) -> Option<String> {
    body.split('&').take(32).filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| decode_component(v))
}

/// Only same-site paths (`/x`, never `//host` or `/\host`), bounded; anything else is `/`.
pub fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.starts_with("/\\")
                && n.len() <= 512
                && n.chars().all(|c| c.is_ascii_graphic()) =>
        {
            n.to_string()
        }
        _ => "/".to_string(),
    }
}

fn lang_code(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Es => "es",
        Lang::Pt => "pt",
        Lang::Fr => "fr",
        Lang::De => "de",
    }
}

/// Render the sign-in page.
pub fn page(lang: Lang, next: &str, error: Option<LoginError>) -> String {
    let title = tr(lang, ["Enter your PIN", "Escribe tu PIN", "Digite seu PIN", "Saisis ton code PIN", "Gib deine PIN ein"]);
    let intro = tr(
        lang,
        [
            "To keep your games private, GrandMentor asks for a PIN the first time you open it on a new device.",
            "Para que tus partidas sigan siendo privadas, GrandMentor pide un PIN la primera vez que lo abres en un dispositivo nuevo.",
            "Para manter suas partidas privadas, o GrandMentor pede um PIN na primeira vez que você o abre em um novo dispositivo.",
            "Pour garder tes parties privées, GrandMentor demande un code PIN la première fois que tu l'ouvres sur un nouvel appareil.",
            "Damit deine Partien privat bleiben, fragt GrandMentor beim ersten Öffnen auf einem neuen Gerät nach einer PIN.",
        ],
    );
    let where_ = tr(
        lang,
        [
            "Find it on the computer that runs GrandMentor: Settings → Use on your phone.",
            "Lo encontrarás en el ordenador donde se ejecuta GrandMentor: Ajustes → Usar en tu móvil.",
            "Ele está no computador que executa o GrandMentor: Configurações → Usar no celular.",
            "Tu le trouveras sur l'ordinateur qui exécute GrandMentor : Réglages → Utiliser sur ton téléphone.",
            "Du findest sie auf dem Computer, auf dem GrandMentor läuft: Einstellungen → Auf dem Handy nutzen.",
        ],
    );
    let label = tr(lang, ["Access PIN", "PIN de acceso", "PIN de acesso", "Code PIN d'accès", "Zugangs-PIN"]);
    let button = tr(lang, ["Unlock", "Entrar", "Entrar", "Déverrouiller", "Entsperren"]);
    let remember = tr(
        lang,
        [
            "This device will stay signed in. You can sign out every device from Settings on your computer.",
            "Este dispositivo seguirá conectado. Puedes cerrar la sesión de todos los dispositivos desde Ajustes en tu ordenador.",
            "Este dispositivo continuará conectado. Você pode desconectar todos os dispositivos nas Configurações do seu computador.",
            "Cet appareil restera connecté. Tu peux déconnecter tous les appareils depuis les Réglages de ton ordinateur.",
            "Dieses Gerät bleibt angemeldet. Du kannst alle Geräte in den Einstellungen auf deinem Computer abmelden.",
        ],
    );
    let error_text = error.map(|e| match e {
        LoginError::Wrong => tr(
            lang,
            [
                "That PIN isn't right. Check it on your computer and try again.",
                "Ese PIN no es correcto. Compruébalo en tu ordenador e inténtalo de nuevo.",
                "Esse PIN não está certo. Confira no computador e tente de novo.",
                "Ce code PIN n'est pas le bon. Vérifie-le sur ton ordinateur et réessaie.",
                "Diese PIN stimmt nicht. Prüfe sie auf deinem Computer und versuch es noch einmal.",
            ],
        ),
        LoginError::Locked(secs) => {
            let min = secs.div_ceil(60).max(1).to_string();
            tr(
                lang,
                [
                    "Too many tries. Wait {min} min and try again.",
                    "Demasiados intentos. Espera {min} min y vuelve a intentarlo.",
                    "Tentativas demais. Espere {min} min e tente de novo.",
                    "Trop d'essais. Attends {min} min et réessaie.",
                    "Zu viele Versuche. Warte {min} Min. und versuch es dann erneut.",
                ],
            )
            .replace("{min}", &min)
        }
        LoginError::Origin => tr(
            lang,
            [
                "Open this page from the address shown on your computer and try again.",
                "Abre esta página desde la dirección que aparece en tu ordenador e inténtalo de nuevo.",
                "Abra esta página pelo endereço mostrado no computador e tente de novo.",
                "Ouvre cette page depuis l'adresse affichée sur ton ordinateur et réessaie.",
                "Öffne diese Seite über die Adresse auf deinem Computer und versuch es erneut.",
            ],
        ),
    });
    let error_html = error_text.map_or(String::new(), |t| format!(r#"<p class="err" role="alert">{}</p>"#, escape(&t)));
    let invalid = if error.is_some() { r#" aria-invalid="true""# } else { "" };
    format!(
        r##"<!doctype html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<meta name="theme-color" content="#1b1e21">
<title>{title} · GrandMentor</title>
<style>
:root{{--bg:#141618;--card:#1f2326;--text:#eef0f2;--muted:#a9b0b7;--accent:#81b64c;--accent-text:#0f1a07;--err:#ff8a80;--line:#343a40}}
@media (prefers-color-scheme: light){{:root{{--bg:#f4f5f6;--card:#fff;--text:#1b1e21;--muted:#5b636b;--accent:#5d8f2c;--accent-text:#fff;--err:#b3261e;--line:#d5d9dd}}}}
*{{box-sizing:border-box}}
body{{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;padding:16px;background:var(--bg);color:var(--text);font:16px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}}
main{{width:100%;max-width:380px;background:var(--card);border:1px solid var(--line);border-radius:16px;padding:28px 24px;text-align:center}}
img{{width:64px;height:64px;border-radius:14px}}
h1{{font-size:1.4rem;margin:12px 0 8px}}
p{{margin:0 0 12px;color:var(--muted)}}
label{{display:block;text-align:left;font-weight:600;margin:16px 0 6px}}
input{{width:100%;font-size:1.6rem;letter-spacing:.3em;text-align:center;padding:12px;border-radius:10px;border:1px solid var(--line);background:var(--bg);color:var(--text)}}
input:focus{{outline:3px solid var(--accent);outline-offset:1px}}
button{{width:100%;margin-top:16px;padding:14px;font-size:1.05rem;font-weight:700;border:0;border-radius:10px;background:var(--accent);color:var(--accent-text);cursor:pointer}}
.err{{color:var(--err);font-weight:600;margin-top:12px}}
.small{{font-size:.85rem;margin-top:16px}}
</style>
</head>
<body>
<main>
<img src="/img/icons/icon-192.png" alt="" width="64" height="64">
<h1>{title}</h1>
<p>{intro}</p>
<p>{where_}</p>
<form method="post" action="/api/access/login">
<input type="hidden" name="next" value="{next}">
<label for="pin">{label}</label>
<input id="pin" name="pin" type="text" inputmode="numeric" autocomplete="one-time-code" maxlength="12" required autofocus{invalid}>
{error_html}
<button type="submit">{button}</button>
</form>
<p class="small">{remember}</p>
</main>
</body>
</html>
"##,
        lang = lang_code(lang),
        title = escape(&title),
        intro = escape(&intro),
        where_ = escape(&where_),
        label = escape(&label),
        button = escape(&button),
        remember = escape(&remember),
        next = escape(next),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_is_same_site_only() {
        assert_eq!(safe_next(Some("/#/play")), "/#/play");
        assert_eq!(safe_next(Some("//evil.com")), "/");
        assert_eq!(safe_next(Some("/\\evil.com")), "/");
        assert_eq!(safe_next(Some("https://evil.com")), "/");
        assert_eq!(safe_next(None), "/");
    }

    #[test]
    fn form_decoding() {
        assert_eq!(form_value("pin=123+456&next=%2F%23%2Fplay", "pin").as_deref(), Some("123 456"));
        assert_eq!(form_value("pin=1&next=%2F%23%2Fplay", "next").as_deref(), Some("/#/play"));
        assert_eq!(decode_component("%zz%4"), "%zz%4");
    }

    #[test]
    fn page_escapes() {
        let html = page(Lang::Es, "/\"><script>", Some(LoginError::Locked(61)));
        assert!(html.contains("Escribe tu PIN"));
        assert!(html.contains("2 min"));
        assert!(!html.contains("\"><script>"));
    }
}
