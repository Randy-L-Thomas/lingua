//! Turn WhatsApp OCR dumps into one chat line per message.

const CHROME: &[&str] = &[
    "yesterday",
    "today",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "sticker",
    "unread",
    "chats",
    "online",
    "forwarded",
];

const CHROME_PHRASES: &[&str] = &[
    "type a message",
    "search or start a new chat",
    "search or start",
    "last seen",
    "today at",
    "tap to",
];

const PULSE_PHRASES: &[&str] = &[
    "ui down",
    "open chat",
    "stdio only",
    "cam-mcp",
    "ws-ops",
    "xsiam-ops",
];

const PULSE_WORDS: &[&str] = &["stdio", "xsiam", "xsiamops", "cammcp", "http", "wsops"];

const SHORT_OK: &[&str] = &[
    "ok", "si", "sí", "no", "hola", "gracias", "jaja", "ja", "yes", "lol", "bye", "okis",
];

/// One OCR line in the chat pane. `cx`/`cy` are 0..1 of the pane (left/top origin).
#[derive(Clone, Debug)]
pub struct OcrSpan {
    pub text: String,
    pub cx: f32,
    pub cy: f32,
}

const ME_X: f32 = 0.56;
const THEM_X: f32 = 0.44;
const WRAP_DY: f32 = 0.07;

pub fn format_wa_spans(spans: &[OcrSpan]) -> String {
    let mut rows: Vec<(f32, f32, String)> = spans
        .iter()
        .filter_map(|s| clean_line(&s.text).map(|t| (s.cy, s.cx, t)))
        .collect();
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Vec::new();
    let mut last_them = String::from("Them");
    let mut i = 0;
    while i < rows.len() {
        let (cy, cx, text) = rows[i].clone();
        let mine = is_mine(cx);
        if !mine && is_speaker_name(&text) {
            last_them = text;
            i += 1;
            continue;
        }
        let mut body = text;
        let mut last_y = cy;
        i += 1;
        while i < rows.len() {
            let (ny, nx, nt) = &rows[i];
            if is_mine(*nx) != mine || *ny - last_y > WRAP_DY {
                break;
            }
            if !mine && is_speaker_name(nt) {
                break;
            }
            body.push(' ');
            body.push_str(nt);
            last_y = *ny;
            i += 1;
        }
        if let Some(keep) = keep_message(&body) {
            let who = if mine { "Me" } else { last_them.as_str() };
            out.push(format!("{who}: {keep}"));
        }
    }
    out.join("\n\n")
}

pub fn format_ocr(plain: &str, spans: &[OcrSpan]) -> String {
    let labeled = format_wa_spans(spans);
    if !labeled.is_empty() {
        labeled
    } else {
        format_wa_ocr(plain)
    }
}

fn is_mine(cx: f32) -> bool {
    if cx >= ME_X {
        true
    } else if cx <= THEM_X {
        false
    } else {
        cx >= 0.5
    }
}

fn clean_line(raw: &str) -> Option<String> {
    let stripped = strip_phrases(raw);
    let mut toks = Vec::new();
    for tok in stripped.split_whitespace() {
        if is_separator(tok) || is_pulse_token(tok) {
            continue;
        }
        toks.push(tok);
    }
    let s = toks.join(" ");
    let s = s
        .trim()
        .trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | ',' | '.' | ';'))
        .trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn is_speaker_name(s: &str) -> bool {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() || words.len() > 3 {
        return false;
    }
    if is_message_start(words[0]) {
        return false;
    }
    let letters: String = s
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect::<String>()
        .to_lowercase();
    if SHORT_OK.contains(&letters.as_str()) {
        return false;
    }
    if letters.len() < 2 || letters.len() > 24 {
        return false;
    }
    if words.len() == 1 {
        return true;
    }
    words.iter().all(|w| {
        let alpha = w.chars().filter(|c| c.is_alphabetic()).count();
        alpha >= 2 && w.chars().next().is_some_and(|c| c.is_uppercase())
    })
}

pub fn looks_like_wa_ocr(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    count_sub(&lower, "yesterday") >= 2
        || count_sub(&lower, "today") >= 2
        || (count_time_tokens(raw) >= 2 && chrome_word_hits(raw) >= 1)
        || has_pulse_chrome(&lower)
}

pub fn format_wa_ocr(raw: &str) -> String {
    let stripped = strip_phrases(raw);
    let mut msgs = Vec::new();
    for para in split_paras(&stripped) {
        take_messages(para, &mut msgs);
    }
    msgs.join("\n\n")
}

fn split_paras(raw: &str) -> Vec<&str> {
    raw.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

fn take_messages(para: &str, msgs: &mut Vec<String>) {
    let mut cur: Vec<&str> = Vec::new();
    let mut skip_pulse = false;
    let flush = |cur: &mut Vec<&str>, msgs: &mut Vec<String>| {
        if cur.is_empty() {
            return;
        }
        let s = cur.join(" ");
        cur.clear();
        if let Some(keep) = keep_message(&s) {
            msgs.push(keep);
        }
    };
    for tok in para.split_whitespace() {
        if is_separator(tok) || is_pulse_token(tok) {
            flush(&mut cur, msgs);
            skip_pulse = is_pulse_token(tok);
            continue;
        }
        if skip_pulse && !is_message_start(tok) {
            continue;
        }
        skip_pulse = false;
        if is_message_start(tok) && !cur.is_empty() && should_split_before_greeting(&cur) {
            flush(&mut cur, msgs);
        }
        cur.push(tok);
    }
    flush(&mut cur, msgs);
}

fn should_split_before_greeting(cur: &[&str]) -> bool {
    let so_far = cur.join(" ");
    keep_message(&so_far).is_none() || cur.len() >= 3
}

fn strip_phrases(raw: &str) -> String {
    let mut out = raw.to_string();
    for phrase in CHROME_PHRASES.iter().chain(PULSE_PHRASES) {
        loop {
            let lower = out.to_ascii_lowercase();
            if let Some(i) = lower.find(phrase) {
                out.replace_range(i..i + phrase.len(), " ");
            } else {
                break;
            }
        }
    }
    out
}

fn is_separator(tok: &str) -> bool {
    is_chrome_word(tok) || is_time_token(tok)
}

fn has_pulse_chrome(lower: &str) -> bool {
    PULSE_PHRASES.iter().any(|p| lower.contains(p)) || lower.split_whitespace().any(is_pulse_token)
}

fn is_pulse_token(tok: &str) -> bool {
    let t = bare(tok);
    let t = t.trim_matches(|c: char| c == ':' || c == '.' || c == '@');
    if PULSE_WORDS.contains(&t) {
        return true;
    }
    if is_pulse_date(t) {
        return true;
    }
    if t == "ms" || t.ends_with("ms") && t.len() > 2 && t[..t.len() - 2].bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    false
}

fn is_pulse_date(t: &str) -> bool {
    let b = t.as_bytes();
    if b.len() < 6 || b.len() > 7 {
        return false;
    }
    let digits_end = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits_end == 0 || digits_end > 2 {
        return false;
    }
    let letters = b[digits_end..].iter().take_while(|c| c.is_ascii_alphabetic()).count();
    if letters != 3 {
        return false;
    }
    let rest = &b[digits_end + 3..];
    rest.len() == 2 && rest.iter().all(|c| c.is_ascii_digit())
}

fn is_message_start(tok: &str) -> bool {
    let t = bare(tok);
    let t = t.trim_matches(|c: char| c == ':' || c == '.');
    matches!(
        t,
        "hola" | "buenas" | "buen" | "hello" | "hey" | "hi" | "good"
    )
}

fn bare(tok: &str) -> String {
    tok.chars()
        .filter(|c| c.is_alphanumeric() || *c == ':' || *c == '.')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn is_chrome_word(tok: &str) -> bool {
    let t = bare(tok);
    let t = t.trim_matches(|c: char| c == ':' || c == '.');
    CHROME.contains(&t)
}

fn is_time_token(tok: &str) -> bool {
    let t = bare(tok);
    if let Some((h, m)) = t.split_once(':').or_else(|| t.split_once('.')) {
        return parse_hour_min(h, m);
    }
    if t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit()) {
        return parse_hour_min(&t[..2], &t[2..]);
    }
    false
}

fn parse_hour_min(h: &str, m: &str) -> bool {
    let Ok(hh) = h.parse::<u32>() else {
        return false;
    };
    let Ok(mm) = m.parse::<u32>() else {
        return false;
    };
    hh <= 23 && mm <= 59 && (h.len() <= 2) && m.len() == 2
}

fn keep_message(s: &str) -> Option<String> {
    let cleaned = s
        .trim()
        .trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | ',' | '.' | ';'))
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return None;
    }
    let letters = cleaned.chars().filter(|c| c.is_alphabetic()).count();
    if letters == 0 {
        return None;
    }
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let simple: String = cleaned
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect::<String>()
        .to_lowercase();
    if SHORT_OK.contains(&simple.as_str()) {
        return Some(cleaned);
    }
    if words.len() >= 3 && letters >= 8 {
        return Some(cleaned);
    }
    if letters >= 18 {
        return Some(cleaned);
    }
    None
}

fn count_sub(hay: &str, needle: &str) -> usize {
    hay.match_indices(needle).count()
}

fn count_time_tokens(raw: &str) -> usize {
    raw.split_whitespace().filter(|t| is_time_token(t)).count()
}

fn chrome_word_hits(raw: &str) -> usize {
    raw.split_whitespace().filter(|t| is_chrome_word(t)).count()
}

#[cfg(test)]
mod tests {
    use super::{format_wa_ocr, format_wa_spans, looks_like_wa_ocr, OcrSpan};

    #[test]
    fn parses_sidebar_jumble_into_two_chat_lines() {
        let raw = "i Yesterday Yesterday Yesterday Yesterday ntregado., Yesterday Yesterday io Gui,. O - Yesterday CISTE., Buenas tardes tiene paquete 1528 1525 hola buenas tardes y gracias' 1525";
        assert!(looks_like_wa_ocr(raw));
        let out = format_wa_ocr(raw);
        assert_eq!(
            out,
            "Buenas tardes tiene paquete\n\nhola buenas tardes y gracias"
        );
    }

    #[test]
    fn keeps_line_broken_messages_and_drops_clocks() {
        let raw = "Yesterday\nBuenas tardes tiene paquete\n15:25\nhola buenas tardes y gracias!\n15:25";
        assert_eq!(
            format_wa_ocr(raw),
            "Buenas tardes tiene paquete\n\nhola buenas tardes y gracias!"
        );
    }

    #[test]
    fn a_real_yesterday_sentence_is_not_wa_ocr() {
        assert!(!looks_like_wa_ocr(
            "Yesterday I picked up the package at the office."
        ));
    }

    #[test]
    fn keeps_short_chat_acks() {
        assert_eq!(format_wa_ocr("ok 15:25"), "ok");
        assert_eq!(format_wa_ocr("gracias"), "gracias");
    }

    #[test]
    fn drops_pulse_strip_leaked_into_whatsapp_ocr() {
        let raw = "Buenas tardes tiene paquete\n\n\
hola buenas Tardes y gracias' 22AUG26 CAM-MCP stdio CAM @.3.35 118 ms • ui down • task ? RAM 2 ms ICE 2 ms ui down ws-ops task ? no\n\n\
TRANSLATE WhatsApp Open chat HTTP — stdio only XSIAM-OPS";
        assert!(
            looks_like_wa_ocr(raw),
            "Translate must re-parse this leftover dump"
        );
        assert_eq!(
            format_wa_ocr(raw),
            "Buenas tardes tiene paquete\n\nhola buenas Tardes y gracias"
        );
    }

    #[test]
    fn drops_pulse_chrome_sitting_above_the_chat() {
        let raw = "22AUG26 CAM-MCP stdio TRANSLATE WhatsApp Open chat \
Buenas tardes tiene paquete hola buenas tardes y gracias";
        assert_eq!(
            format_wa_ocr(raw),
            "Buenas tardes tiene paquete\n\nhola buenas tardes y gracias"
        );
    }

    #[test]
    fn a_normal_translate_request_is_not_wa_ocr() {
        assert!(!looks_like_wa_ocr(
            "Can you translate this package note for me tomorrow?"
        ));
    }

    #[test]
    fn right_bubble_is_me_left_is_them() {
        let out = format_wa_spans(&[
            OcrSpan {
                text: "Buenas tardes tiene paquete".into(),
                cx: 0.28,
                cy: 0.20,
            },
            OcrSpan {
                text: "ok".into(),
                cx: 0.80,
                cy: 0.40,
            },
        ]);
        assert_eq!(
            out,
            "Them: Buenas tardes tiene paquete\n\nMe: ok"
        );
    }

    #[test]
    fn group_name_labels_following_left_bubbles() {
        let out = format_wa_spans(&[
            OcrSpan {
                text: "Maria".into(),
                cx: 0.22,
                cy: 0.10,
            },
            OcrSpan {
                text: "Buenas tardes tiene paquete".into(),
                cx: 0.30,
                cy: 0.18,
            },
            OcrSpan {
                text: "hola buenas tardes y gracias".into(),
                cx: 0.29,
                cy: 0.42,
            },
            OcrSpan {
                text: "ok".into(),
                cx: 0.82,
                cy: 0.60,
            },
        ]);
        assert_eq!(
            out,
            "Maria: Buenas tardes tiene paquete\n\nMaria: hola buenas tardes y gracias\n\nMe: ok"
        );
    }

    fn fixture(name: &str) -> &'static str {
        match name {
            "sidebar_jumble.txt" => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/wa_ocr/sidebar_jumble.txt"
                ))
            }
            "sidebar_jumble.expected.txt" => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/wa_ocr/sidebar_jumble.expected.txt"
                ))
            }
            "pulse_leak.txt" => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/wa_ocr/pulse_leak.txt"
                ))
            }
            "pulse_leak.expected.txt" => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/wa_ocr/pulse_leak.expected.txt"
                ))
            }
            "spans_me_them.txt" => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/wa_ocr/spans_me_them.txt"
                ))
            }
            "spans_me_them.expected.txt" => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/wa_ocr/spans_me_them.expected.txt"
                ))
            }
            _ => panic!("unknown fixture {name}"),
        }
    }

    fn load_spans(raw: &str) -> Vec<OcrSpan> {
        raw.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| {
                let mut parts = l.splitn(3, ' ');
                let cx: f32 = parts.next().unwrap().parse().unwrap();
                let cy: f32 = parts.next().unwrap().parse().unwrap();
                let text = parts.next().unwrap().to_string();
                OcrSpan { text, cx, cy }
            })
            .collect()
    }

    fn norm(s: &str) -> String {
        s.replace("\r\n", "\n").trim().to_string()
    }

    #[test]
    fn golden_wa_ocr_fixtures() {
        assert_eq!(
            norm(&format_wa_ocr(fixture("sidebar_jumble.txt"))),
            norm(fixture("sidebar_jumble.expected.txt"))
        );
        assert_eq!(
            norm(&format_wa_ocr(fixture("pulse_leak.txt"))),
            norm(fixture("pulse_leak.expected.txt"))
        );
        assert_eq!(
            norm(&format_wa_spans(&load_spans(fixture("spans_me_them.txt")))),
            norm(fixture("spans_me_them.expected.txt"))
        );
    }

    #[test]
    fn wraps_nearby_same_side_lines_into_one_message() {
        let out = format_wa_spans(&[
            OcrSpan {
                text: "Buenas tardes".into(),
                cx: 0.28,
                cy: 0.20,
            },
            OcrSpan {
                text: "tiene paquete".into(),
                cx: 0.30,
                cy: 0.24,
            },
        ]);
        assert_eq!(out, "Them: Buenas tardes tiene paquete");
    }
}
