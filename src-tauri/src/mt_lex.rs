//! Instant Mexican Spanish ↔ American English for chat. Phrase-first, then words.
//! Built-in tables live in `lex/*.tsv`. User rows in the Pulse config dir overlay them.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

pub(crate) const USER_LEX_ES_EN: &str = "lex-es-en.tsv";
pub(crate) const USER_LEX_EN_ES: &str = "lex-en-es.tsv";

const BUILTIN_ES_EN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/lex/es-en.tsv"));
const BUILTIN_EN_ES: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/lex/en-es.tsv"));

fn parse_tsv_into(map: &mut HashMap<String, String>, raw: &str, overwrite: bool) {
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((src, dst)) = line.split_once('\t') else {
            continue;
        };
        let k = fold_key(src);
        if k.is_empty() {
            continue;
        }
        let v = dst.trim().to_string();
        if overwrite {
            map.insert(k, v);
        } else {
            map.entry(k).or_insert(v);
        }
    }
}

fn parse_tsv(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    parse_tsv_into(&mut map, raw, false);
    map
}

pub(crate) fn load_lex_table(builtin: &str, overlay_path: &Path) -> HashMap<String, String> {
    let mut map = parse_tsv(builtin);
    if let Ok(raw) = fs::read_to_string(overlay_path) {
        parse_tsv_into(&mut map, &raw, true);
    }
    map
}

fn runtime_table(builtin: &str, filename: &str) -> HashMap<String, String> {
    if cfg!(test) {
        parse_tsv(builtin)
    } else {
        load_lex_table(builtin, &crate::config::user_config_dir().join(filename))
    }
}

fn es_en() -> &'static HashMap<String, String> {
    static T: OnceLock<HashMap<String, String>> = OnceLock::new();
    T.get_or_init(|| runtime_table(BUILTIN_ES_EN, USER_LEX_ES_EN))
}

fn en_es() -> &'static HashMap<String, String> {
    static T: OnceLock<HashMap<String, String>> = OnceLock::new();
    T.get_or_init(|| runtime_table(BUILTIN_EN_ES, USER_LEX_EN_ES))
}

pub(crate) fn es_en_size() -> usize {
    es_en().len()
}

fn table<'a>(
    from: &str,
    to: &str,
    es_en: &'a HashMap<String, String>,
    en_es: &'a HashMap<String, String>,
) -> &'a HashMap<String, String> {
    if from == "es" && to == "en" {
        es_en
    } else {
        en_es
    }
}

fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' => 'a',
            'é' | 'è' => 'e',
            'í' | 'ì' => 'i',
            'ó' | 'ò' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            'Á' | 'À' => 'a',
            'É' | 'È' => 'e',
            'Í' | 'Ì' => 'i',
            'Ó' | 'Ò' => 'o',
            'Ú' | 'Ù' | 'Ü' => 'u',
            'Ñ' => 'n',
            '¿' | '¡' => ' ',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

pub(crate) fn split_speaker(line: &str) -> (Option<String>, &str) {
    let Some((who, rest)) = line.split_once(':') else {
        return (None, line);
    };
    let who = who.trim();
    if who.is_empty() || who.len() > 24 || who.chars().any(|c| c == ',' || c == '?' || c == '!') {
        return (None, line);
    }
    let words = who.split_whitespace().count();
    if words == 0 || words > 3 {
        return (None, line);
    }
    (Some(who.to_string()), rest.trim())
}

fn fold_key(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in fold(s).chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            space = false;
        } else if !space && !out.is_empty() {
            out.push(' ');
            space = true;
        }
    }
    out.trim().to_string()
}

fn lookup<'a>(tab: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    let k = fold_key(key);
    if k.is_empty() {
        return None;
    }
    tab.get(&k).map(String::as_str)
}

fn translate_body(
    from: &str,
    to: &str,
    body: &str,
    es_en: &HashMap<String, String>,
    en_es: &HashMap<String, String>,
) -> String {
    let body = body.trim();
    if body.is_empty() {
        return String::new();
    }
    let tab = table(from, to, es_en, en_es);
    if let Some(hit) = lookup(tab, body) {
        if body.split_whitespace().count() == 1 {
            return cap_first(hit);
        }
        return hit.to_string();
    }
    let toks: Vec<&str> = body.split_whitespace().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < toks.len() {
        let mut hit = None;
        let max = 6.min(toks.len() - i);
        for n in (1..=max).rev() {
            let chunk = toks[i..i + n].join(" ");
            if let Some(t) = lookup(tab, &chunk) {
                hit = Some((n, t.to_string()));
                break;
            }
        }
        if let Some((n, t)) = hit {
            out.push(t);
            i += n;
            continue;
        }
        let (core, trail) = peel(toks[i]);
        if let Some(t) = lookup(tab, &core) {
            out.push(format!("{t}{trail}"));
        } else {
            out.push(toks[i].to_string());
        }
        i += 1;
    }
    let s = out.join(" ");
    cap_first(&s)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, 'á' | 'é' | 'í' | 'ó' | 'ú' | 'ñ' | 'ü' | 'Á' | 'É' | 'Í' | 'Ó' | 'Ú' | 'Ñ' | 'Ü')
}

fn peel(tok: &str) -> (String, String) {
    let start = tok
        .char_indices()
        .find(|(_, c)| is_word_char(*c))
        .map(|(i, _)| i)
        .unwrap_or(tok.len());
    let end = tok
        .char_indices()
        .rev()
        .find(|(_, c)| is_word_char(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(start);
    if start >= end {
        return (tok.to_string(), String::new());
    }
    (tok[start..end].to_string(), tok[end..].to_string())
}

fn cap_first(s: &str) -> String {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::new();
    out.extend(first.to_uppercase());
    out.extend(chars);
    out
}

fn translate_with(
    from: &str,
    to: &str,
    src: &str,
    es_en: &HashMap<String, String>,
    en_es: &HashMap<String, String>,
) -> String {
    let mut lines = Vec::new();
    for line in src.split('\n') {
        if line.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let (who, body) = split_speaker(line);
        let t = translate_body(from, to, body, es_en, en_es);
        if let Some(who) = who {
            lines.push(if t.is_empty() {
                format!("{who}:")
            } else {
                format!("{who}: {t}")
            });
        } else {
            lines.push(t);
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

pub(crate) fn translate_lex_with_overlay(
    from: &str,
    to: &str,
    src: &str,
    overlay_dir: &Path,
) -> String {
    let es = load_lex_table(BUILTIN_ES_EN, &overlay_dir.join(USER_LEX_ES_EN));
    let en = load_lex_table(BUILTIN_EN_ES, &overlay_dir.join(USER_LEX_EN_ES));
    translate_with(from, to, src, &es, &en)
}

pub fn translate_lex(from: &str, to: &str, src: &str) -> String {
    translate_with(from, to, src, es_en(), en_es())
}

#[cfg(test)]
mod tests {
    use super::translate_lex;

    #[test]
    fn phrases_and_speaker_labels() {
        assert_eq!(
            translate_lex("es", "en", "Buenas tardes tiene paquete"),
            "Good afternoon, you have a package"
        );
        assert_eq!(
            translate_lex("es", "en", "Me: hola\n\nThem: gracias"),
            "Me: Hello\n\nThem: Thank you"
        );
        assert_eq!(
            translate_lex("es", "en", "Maria: buenas tardes"),
            "Maria: Good afternoon"
        );
    }

    #[test]
    fn unknown_tokens_stay() {
        let out = translate_lex("es", "en", "CAM-MCP 22AUG26");
        assert!(out.contains("CAM-MCP"));
        assert!(out.contains("22AUG26"));
    }

    #[test]
    fn english_back_to_spanish() {
        assert_eq!(translate_lex("en", "es", "thank you"), "gracias");
        assert_eq!(
            translate_lex("en", "es", "Good afternoon"),
            "Buenas tardes"
        );
    }

    #[test]
    fn loads_a_bigger_file_table() {
        assert!(
            super::es_en_size() >= 300,
            "es→en table too small: {}",
            super::es_en_size()
        );
        assert_eq!(translate_lex("es", "en", "El perro"), "The dog");
        assert_eq!(
            translate_lex("es", "en", "Te mando mi celular"),
            "I'll send you my cell phone"
        );
    }

    #[test]
    fn el_gato_is_the_cat_not_the_gato() {
        assert_eq!(translate_lex("es", "en", "El Gato"), "The cat");
        assert_eq!(translate_lex("es", "en", "el gato"), "The cat");
        assert_eq!(translate_lex("en", "es", "The Cat"), "El gato");
    }

    #[test]
    fn articles_translate_even_before_unknown_nouns() {
        assert_eq!(translate_lex("es", "en", "las"), "The");
        assert_eq!(translate_lex("es", "en", "Las"), "The");
        assert_eq!(translate_lex("es", "en", "las casas"), "The houses");
        assert_eq!(translate_lex("es", "en", "Las casas"), "The houses");
        assert_eq!(translate_lex("es", "en", "las Quetzalito"), "The Quetzalito");
        assert_eq!(translate_lex("es", "en", "El Quetzalito"), "The Quetzalito");
        assert_eq!(translate_lex("es", "en", "El Gato"), "The cat");
        assert_eq!(translate_lex("es", "en", "les"), "Them");
    }

    #[test]
    fn punctuated_ocr_line_still_translates() {
        assert_eq!(
            translate_lex("es", "en", "Them: Buenas tardes, ¿tiene paquete?"),
            "Them: Good afternoon, you have a package"
        );
    }

    #[test]
    fn user_tsv_adds_nonce_and_overrides_builtin() {
        let dir = std::env::temp_dir().join(format!(
            "pulse-lex-overlay-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("lex-es-en.tsv"),
            "quetzalito\tnonce bird\ngato\thousecat\n",
        )
        .unwrap();

        assert_eq!(
            super::translate_lex_with_overlay("es", "en", "El Quetzalito", &dir),
            "The nonce bird"
        );
        assert_eq!(
            super::translate_lex_with_overlay("es", "en", "El Gato", &dir),
            "The housecat"
        );

        let missing = dir.join("no-such-overlay-dir");
        assert_eq!(
            super::translate_lex_with_overlay("es", "en", "El Gato", &missing),
            "The cat"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
