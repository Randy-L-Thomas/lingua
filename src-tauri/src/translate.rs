use crate::mt_seq2seq::Seq2SeqMt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type ProgressFn = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    entries: HashMap<String, String>,
}

fn cache_path() -> PathBuf {
    crate::config::user_config_dir().join("mt-cache.json")
}

fn load_cache() -> Cache {
    fs::read_to_string(cache_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cache(cache: &Cache) {
    if let Ok(raw) = serde_json::to_string_pretty(cache) {
        let _ = fs::create_dir_all(crate::config::user_config_dir());
        let _ = fs::write(cache_path(), raw);
    }
}

fn key(from: &str, to: &str, text: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"es-MX|en-US|opus-v2|");
    h.update(from.as_bytes());
    h.update(b"|");
    h.update(to.as_bytes());
    h.update(b"|");
    h.update(text.as_bytes());
    hex::encode(h.finalize())
}

#[derive(Serialize)]
pub struct TranslateOut {
    pub text: String,
    pub cached: bool,
    pub engine: String,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// UI default: Opus-MT. `false` is the explicit lexicon path.
pub fn default_use_opus() -> bool {
    true
}

fn is_opus_unavailable(err: &str) -> bool {
    let l = err.to_ascii_lowercase();
    l.contains("could not download")
        || l.contains("files missing")
        || l.contains("could not load")
        || l.contains("could not init")
        || l.contains("opus-mt http")
}

pub async fn translate(
    from: &str,
    to: &str,
    source: &str,
    llm: bool,
    on_progress: Option<ProgressFn>,
) -> Result<TranslateOut, String> {
    translate_inner(from, to, source, llm, None, on_progress, true).await
}

#[cfg(test)]
pub async fn translate_with_engine(
    from: &str,
    to: &str,
    source: &str,
    llm: bool,
    engine: &dyn Seq2SeqMt,
) -> Result<TranslateOut, String> {
    translate_inner(from, to, source, llm, Some(engine), None, false).await
}

async fn translate_inner(
    from: &str,
    to: &str,
    source: &str,
    llm: bool,
    engine: Option<&dyn Seq2SeqMt>,
    on_progress: Option<ProgressFn>,
    use_disk_cache: bool,
) -> Result<TranslateOut, String> {
    let src = prepare_source(source);
    if src.is_empty() {
        return Ok(TranslateOut {
            text: String::new(),
            cached: true,
            engine: "empty".into(),
            model: None,
            warning: None,
        });
    }
    let from = if from == "auto" {
        detect_lang(&src)
    } else {
        normalize_lang(from)
    };
    let to = normalize_lang(to);
    if from == to {
        return Ok(TranslateOut {
            text: src,
            cached: true,
            engine: "same".into(),
            model: None,
            warning: None,
        });
    }
    if !llm {
        let out = crate::mt_lex::translate_lex(from, to, &src);
        return Ok(TranslateOut {
            text: out,
            cached: false,
            engine: "lex".into(),
            model: None,
            warning: None,
        });
    }
    let model = crate::mt_seq2seq::model_id(from, to);
    if use_disk_cache {
        let k = key(from, to, &src);
        let cache = load_cache();
        if let Some(hit) = cache.entries.get(&k) {
            if is_usable_translation(hit, &src) {
                return Ok(TranslateOut {
                    text: hit.clone(),
                    cached: true,
                    engine: "cache".into(),
                    model: Some(model),
                    warning: None,
                });
            }
        }
    }
    let from_s = from.to_string();
    let to_s = to.to_string();
    let src_s = src.clone();
    let out = if let Some(engine) = engine {
        let mut progress = |msg: &str| {
            if let Some(p) = &on_progress {
                p(msg);
            }
        };
        match crate::mt_seq2seq::run_seq2seq(engine, from, to, &src, &mut progress) {
            Ok(t) => t,
            Err(e) if is_opus_unavailable(&e) => {
                return Ok(lex_after_opus(from, to, &src, &e));
            }
            Err(e) => return Err(e),
        }
    } else {
        let progress = on_progress.clone();
        match tokio::task::spawn_blocking(move || {
            let mut emit = |msg: &str| {
                if let Some(p) = &progress {
                    p(msg);
                }
            };
            crate::mt_opus::translate(&from_s, &to_s, &src_s, &mut emit)
        })
        .await
        .map_err(|e| e.to_string())?
        {
            Ok(t) => t,
            Err(e) if is_opus_unavailable(&e) => {
                return Ok(lex_after_opus(from, to, &src, &e));
            }
            Err(e) => return Err(e),
        }
    };
    if !is_usable_translation(&out, &src) {
        return Err("Opus-MT repeated the source".into());
    }
    if use_disk_cache {
        let k = key(from, to, &src);
        let mut cache = load_cache();
        cache.entries.insert(k, out.clone());
        save_cache(&cache);
    }
    Ok(TranslateOut {
        text: out,
        cached: false,
        engine: "opus".into(),
        model: Some(model),
        warning: None,
    })
}

fn lex_after_opus(from: &str, to: &str, src: &str, err: &str) -> TranslateOut {
    TranslateOut {
        text: crate::mt_lex::translate_lex(from, to, src),
        cached: false,
        engine: "lex".into(),
        model: None,
        warning: Some(err.to_string()),
    }
}

fn lang_hit_counts(text: &str) -> (usize, usize) {
    let t = text.to_lowercase();
    let marks = t.chars().filter(|c| "áéíóúñü¿¡".contains(*c)).count();
    let es = [
        "qué",
        "hola",
        "gracias",
        "por favor",
        "buenos",
        "usted",
        "está",
        "también",
        "mañana",
    ];
    let en = ["the ", " and ", " you ", " that ", " with ", " this "];
    let es_hits = es.iter().filter(|w| t.contains(*w)).count() + marks;
    let en_hits = en.iter().filter(|w| t.contains(*w)).count();
    (es_hits, en_hits)
}

pub fn detect_lang(text: &str) -> &'static str {
    detect_lang_confident(text).unwrap_or_else(|| {
        let (es_hits, en_hits) = lang_hit_counts(text);
        if es_hits > en_hits {
            "es"
        } else if en_hits > 0 {
            "en"
        } else {
            "es"
        }
    })
}

/// Spanish marks/words vs English function words. `None` when short or ambiguous
/// so the UI will not auto-swap (plain `detect_lang` still defaults unsure text to `"es"`).
pub fn detect_lang_confident(text: &str) -> Option<&'static str> {
    let trimmed = text.trim();
    if trimmed.chars().count() < 12 {
        return None;
    }
    let padded = format!(" {} ", trimmed.to_lowercase());
    let (mut es_hits, mut en_hits) = lang_hit_counts(trimmed);
    let es_func = [
        " el ", " la ", " de ", " que ", " por ", " con ", " los ", " las ", " una ", " para ",
    ];
    let en_func = [" is ", " are ", " of ", " to ", " in ", " for "];
    es_hits += es_func.iter().filter(|w| padded.contains(*w)).count();
    en_hits += en_func.iter().filter(|w| padded.contains(*w)).count();
    const MIN_HITS: usize = 2;
    if es_hits >= MIN_HITS && es_hits > en_hits {
        Some("es")
    } else if en_hits >= MIN_HITS && en_hits > es_hits {
        Some("en")
    } else {
        None
    }
}

fn normalize_lang(code: &str) -> &'static str {
    match code.trim().to_ascii_lowercase().as_str() {
        "en" | "en-us" | "en_us" | "us" => "en",
        "es" | "es-mx" | "es_mx" | "mx" => "es",
        other if other.starts_with("en") => "en",
        _ => "es",
    }
}

fn prepare_source(source: &str) -> String {
    let src = source.trim();
    if crate::ocr_text::looks_like_wa_ocr(src) {
        let formatted = crate::ocr_text::format_wa_ocr(src);
        if !formatted.is_empty() {
            return formatted;
        }
    }
    src.to_string()
}

fn lang_name(code: &str) -> &'static str {
    match normalize_lang(code) {
        "es" => "Mexican Spanish",
        "en" => "American English",
        _ => "the target language",
    }
}

fn mt_prompt(from: &str, to: &str, src: &str) -> String {
    format!(
        "You are a translator for Mexican Spanish and American English chat messages.\n\
Translate from {} to {}.\n\
Mexican Spanish: tú for casual chat, usted if the source is clearly formal. \
Mexico vocabulary (computadora, celular, carro, platicar, departamento). \
Never Spain-only forms (vosotros, coche, ordenador, móvil, piso).\n\
American English: US spelling and vocabulary (color, truck, apartment, cell phone, elevator, soccer). \
Never British-only forms (colour, lorry, flat, mobile, lift, football).\n\
If the source has multiple chat lines, translate each non-empty line onto its own line. \
Keep a blank line between messages. Do not merge them into one paragraph or invent extra sentences.\n\
If a line starts with a speaker label (Me:, Them:, or a name then a colon), copy that label exactly and translate only the words after it.\n\
Output ONLY the translation. No quotes, notes, or extra lines.\n\n{}",
        lang_name(from),
        lang_name(to),
        src
    )
}

fn mt_retry_prompt(from: &str, to: &str, src: &str) -> String {
    format!(
        "{}\n\nDo not repeat the source text. Output only the translation, nothing else.",
        mt_prompt(from, to, src)
    )
}

pub(crate) fn is_usable_translation(out: &str, src: &str) -> bool {
    let t = out.trim();
    !t.is_empty() && !t.eq_ignore_ascii_case(src.trim())
}

pub(crate) fn should_retry_mt(cleaned: &str, src: &str) -> bool {
    !is_usable_translation(cleaned, src)
}

#[allow(dead_code)]
async fn mt_ollama(
    client: &reqwest::Client,
    base: &str,
    from: &str,
    to: &str,
    src: &str,
) -> Result<(String, String), String> {
    let model = pick_model(client, base).await?;
    let raw = generate(client, base, &model, &mt_prompt(from, to, src), 0.0).await?;
    let out = clean_translation(&raw, src);
    if is_usable_translation(&out, src) {
        return Ok((out, model));
    }
    let raw2 = generate(client, base, &model, &mt_retry_prompt(from, to, src), 0.0).await?;
    let out2 = clean_translation(&raw2, src);
    if is_usable_translation(&out2, src) {
        return Ok((out2, model));
    }
    if out.is_empty() && out2.is_empty() {
        return Err(format!("empty translation from {model}"));
    }
    Err(format!("model {model} repeated the source"))
}

#[allow(dead_code)]
async fn enrich_ollama(
    client: &reqwest::Client,
    base: &str,
    from: &str,
    to: &str,
    src: &str,
    draft: &str,
) -> Result<String, String> {
    let model = pick_model(client, base).await?;
    let prompt = format!(
        "Improve this {} translation of a chat message so it sounds natural for Mexico/US. \
Output only the rewritten translation.\n\nSource ({}): {}\nDraft: {}",
        lang_name(to),
        lang_name(from),
        src,
        draft
    );
    let raw = generate(client, base, &model, &prompt, 0.3).await?;
    Ok(clean_translation(&raw, src))
}

fn is_reasoning_model(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.contains("deepseek-r1")
        || l.contains("r1-")
        || l.contains(":r1")
        || l.contains("thinking")
        || l.contains("reason")
}

fn is_coder_model(name: &str) -> bool {
    name.to_ascii_lowercase().contains("coder")
}

fn is_mt_model(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.contains("translate")
        || l.contains("nllb")
        || l.contains("madlad")
        || l.contains("m2m")
        || l.contains("opus-mt")
        || l.contains("-mt")
        || l.contains("mt:")
}

pub(crate) fn choose_mt_model(names: &[String]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    if let Some(n) = names.iter().find(|n| is_mt_model(n)) {
        return Some(n.clone());
    }
    let prefer = [
        "llama3.2:1b",
        "llama3.2:3b",
        "qwen2.5:1.5b",
        "qwen2.5:3b",
        "gemma2:2b",
        "llama3.1:8b",
        "qwen2.5:7b",
        "qwen2.5:14b",
    ];
    for p in prefer {
        if let Some(n) = names.iter().find(|n| {
            (n.as_str() == p || n.starts_with(p)) && !is_coder_model(n) && !is_reasoning_model(n)
        }) {
            return Some(n.clone());
        }
    }
    let instructish = |n: &str| {
        let l = n.to_ascii_lowercase();
        l.contains("qwen")
            || l.contains("llama")
            || l.contains("gemma")
            || l.contains("mistral")
            || l.contains("phi")
    };
    if let Some(n) = names
        .iter()
        .find(|n| instructish(n) && !is_reasoning_model(n) && !is_coder_model(n))
    {
        return Some(n.clone());
    }
    if let Some(n) = names
        .iter()
        .find(|n| !is_reasoning_model(n) && !is_coder_model(n))
    {
        return Some(n.clone());
    }
    if let Some(n) = names.iter().find(|n| !is_reasoning_model(n)) {
        return Some(n.clone());
    }
    names.first().cloned()
}

struct MtModelPick {
    model: String,
    at: Instant,
}

static MT_MODEL: Mutex<Option<MtModelPick>> = Mutex::new(None);
const MT_MODEL_TTL: Duration = Duration::from_secs(5 * 60);

fn cached_mt_model() -> Option<String> {
    let guard = MT_MODEL.lock().ok()?;
    let hit = guard.as_ref()?;
    if hit.at.elapsed() < MT_MODEL_TTL {
        Some(hit.model.clone())
    } else {
        None
    }
}

fn store_mt_model(model: String) {
    if let Ok(mut guard) = MT_MODEL.lock() {
        *guard = Some(MtModelPick {
            model,
            at: Instant::now(),
        });
    }
}

fn invalidate_mt_model() {
    if let Ok(mut guard) = MT_MODEL.lock() {
        *guard = None;
    }
}

async fn pick_model(client: &reqwest::Client, base: &str) -> Result<String, String> {
    if let Some(model) = cached_mt_model() {
        return Ok(model);
    }
    let names = crate::ollama::list_models(client, base)
        .await
        .map_err(|e| {
            invalidate_mt_model();
            map_ollama_err(e, base)
        })?;
    let model = choose_mt_model(&names)
        .ok_or_else(|| "no Ollama models — pull one for local MT".to_string())?;
    store_mt_model(model.clone());
    Ok(model)
}

fn map_ollama_err(err: impl std::fmt::Display, ollama_url: &str) -> String {
    crate::ollama::map_connect_err(err, ollama_url)
}

fn clean_translation(raw: &str, src: &str) -> String {
    let mut t = raw.trim().to_string();
    if let Some(rest) = t.strip_prefix("<think>") {
        t = if let Some((_, after)) = rest.split_once("</think>") {
            after.trim().to_string()
        } else {
            String::new()
        };
    }
    for prefix in [
        "Translation:",
        "Translated:",
        "American English:",
        "Mexican Spanish:",
        "English:",
        "Spanish:",
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim().to_string();
        }
    }
    t = t
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| t.strip_prefix('“').and_then(|s| s.strip_suffix('”')))
        .map(str::trim)
        .unwrap_or(&t)
        .to_string();
    let src = src.trim();
    if t.eq_ignore_ascii_case(src) {
        return String::new();
    }
    t = t
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.is_empty()
                || (!line.starts_with("Translate from") && !line.starts_with("Output ONLY"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    while t.contains("\n\n\n") {
        t = t.replace("\n\n\n", "\n\n");
    }
    if t.eq_ignore_ascii_case(src) {
        String::new()
    } else {
        t
    }
}

async fn generate(
    client: &reqwest::Client,
    base: &str,
    model: &str,
    prompt: &str,
    temperature: f32,
) -> Result<String, String> {
    let url = format!("{}/api/generate", base.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "options": { "temperature": temperature, "num_predict": 512 }
    });
    let res = client.post(url).json(&body).send().await.map_err(|e| {
        invalidate_mt_model();
        map_ollama_err(e, base)
    })?;
    if !res.status().is_success() {
        invalidate_mt_model();
        return Err(format!("Ollama HTTP {}", res.status()));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(v.get("response")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        choose_mt_model, clean_translation, detect_lang, detect_lang_confident,
        is_usable_translation, lang_name, mt_prompt, mt_retry_prompt, normalize_lang,
        should_retry_mt,
    };

    #[test]
    fn detects_spanish_from_marks_and_words() {
        assert_eq!(detect_lang("¿Cómo estás? Gracias por el mensaje."), "es");
    }

    #[test]
    fn detects_english_from_common_words() {
        assert_eq!(detect_lang("The cat sat with you and that box."), "en");
    }

    #[test]
    fn confident_detect_spanish_vs_english() {
        assert_eq!(
            detect_lang_confident("¿Cómo estás? Gracias por el mensaje."),
            Some("es")
        );
        assert_eq!(
            detect_lang_confident("Hola, gracias por el mensaje de ayer."),
            Some("es")
        );
        assert_eq!(
            detect_lang_confident("The cat sat with you and that box."),
            Some("en")
        );
        assert_eq!(
            detect_lang_confident("Please look at this with care today."),
            Some("en")
        );
    }

    #[test]
    fn confident_detect_skips_short_and_ambiguous() {
        assert_eq!(detect_lang_confident("Hola"), None);
        assert_eq!(detect_lang_confident("Hi"), None);
        assert_eq!(detect_lang_confident("ok"), None);
        assert_eq!(detect_lang_confident("Hello there"), None);
        assert_eq!(detect_lang_confident("xyz abc def ghi jkl"), None);
        assert_eq!(
            detect_lang_confident("Them: Buenas tardes tiene paquete\n\nMe: hola gracias"),
            Some("es")
        );
        assert_eq!(detect_lang("Hello"), "es");
        assert_eq!(detect_lang("ok"), "es");
    }

    #[test]
    fn normalizes_mx_and_us_codes() {
        assert_eq!(normalize_lang("es-MX"), "es");
        assert_eq!(normalize_lang("en-US"), "en");
        assert_eq!(lang_name("es"), "Mexican Spanish");
        assert_eq!(lang_name("en"), "American English");
    }

    #[test]
    fn prompt_is_mexico_and_us() {
        let p = mt_prompt("es", "en", "Hola");
        assert!(p.contains("Mexican Spanish"));
        assert!(p.contains("American English"));
        assert!(p.contains("vosotros"));
        assert!(p.contains("lorry"));
        assert!(p.contains("Hola"));
        assert!(p.contains("each non-empty line"));
        assert!(p.contains("speaker label"));
    }

    #[test]
    fn prefers_translate_model_then_instruct_not_r1_or_coder() {
        let names = vec![
            "deepseek-r1:14b".into(),
            "qwen2.5-coder:14b".into(),
            "llama3.1:8b".into(),
        ];
        assert_eq!(choose_mt_model(&names).as_deref(), Some("llama3.1:8b"));
        let only_specialized = vec!["deepseek-r1:14b".into(), "qwen2.5-coder:14b".into()];
        assert_eq!(
            choose_mt_model(&only_specialized).as_deref(),
            Some("qwen2.5-coder:14b")
        );
        let mt = vec!["llama3.2:3b".into(), "nllb-translate:latest".into()];
        assert_eq!(
            choose_mt_model(&mt).as_deref(),
            Some("nllb-translate:latest")
        );
    }

    #[test]
    fn prefers_tiny_instruct_before_large() {
        let names = vec![
            "llama3.1:8b".into(),
            "qwen2.5:3b".into(),
            "llama3.2:1b".into(),
        ];
        assert_eq!(choose_mt_model(&names).as_deref(), Some("llama3.2:1b"));
        let seven = vec!["qwen2.5:7b".into(), "llama3.2:1b".into()];
        assert_eq!(choose_mt_model(&seven).as_deref(), Some("llama3.2:1b"));
        let fourteen = vec!["qwen2.5:14b".into(), "gemma2:2b".into()];
        assert_eq!(choose_mt_model(&fourteen).as_deref(), Some("gemma2:2b"));
        let mt_still_wins = vec!["llama3.1:8b".into(), "madlad400:3b".into()];
        assert_eq!(
            choose_mt_model(&mt_still_wins).as_deref(),
            Some("madlad400:3b")
        );
    }

    #[test]
    fn strips_think_tags_and_labels() {
        let raw = "<think>plan</think>\nTranslation: Hello\n";
        assert_eq!(clean_translation(raw, "Hola"), "Hello");
    }

    #[test]
    fn prepare_source_keeps_labeled_chat_for_opus() {
        let src = "Me: El Gato\n\nThem: las casas\n\nThem: Buenas tardes, ¿tiene paquete?";
        assert_eq!(super::prepare_source(src), src);
    }

    #[test]
    fn prepare_source_strips_pulse_leftover_without_reocr() {
        let raw = "Buenas tardes tiene paquete\n\n\
hola buenas Tardes y gracias' 22AUG26 CAM-MCP stdio CAM @.3.35 118 ms • ui down • task ? RAM 2 ms ICE 2 ms ui down ws-ops task ? no\n\n\
TRANSLATE WhatsApp Open chat HTTP — stdio only XSIAM-OPS";
        assert_eq!(
            super::prepare_source(raw),
            "Buenas tardes tiene paquete\n\nhola buenas Tardes y gracias"
        );
    }

    #[test]
    fn keeps_blank_line_between_chat_messages() {
        let raw = "Good afternoon, you have a package.\n\nHello, good afternoon and thanks!";
        assert_eq!(clean_translation(raw, "x"), raw);
    }

    #[test]
    fn echo_and_empty_are_not_usable_or_cacheable() {
        assert_eq!(clean_translation("Hola", "Hola"), "");
        assert_eq!(clean_translation("hola", "Hola"), "");
        assert_eq!(clean_translation("<think>still thinking", "Hola"), "");
        assert_eq!(clean_translation("  Hola  ", "Hola"), "");
        assert!(!is_usable_translation("", "Hola"));
        assert!(!is_usable_translation("Hola", "Hola"));
        assert!(!is_usable_translation("hola", "Hola"));
        assert!(is_usable_translation("Hello", "Hola"));
        assert!(should_retry_mt("Hola", "Hola"));
        assert!(should_retry_mt("", "Hola"));
        assert!(!should_retry_mt("Hello", "Hola"));
    }

    #[test]
    fn retry_prompt_forbids_repeating_source() {
        let p = mt_retry_prompt("es", "en", "Hola");
        assert!(p.contains("Do not repeat the source"));
        assert!(p.contains("Output only the translation"));
        assert!(p.contains("Hola"));
    }

    #[test]
    fn connection_errors_are_plain() {
        assert_eq!(
            super::map_ollama_err(
                "error sending request for url: tcp connect error: connection refused",
                "http://127.0.0.1:11434"
            ),
            "Ollama not running on 127.0.0.1:11434"
        );
        assert_eq!(
            super::map_ollama_err("connection refused", "http://192.168.1.5:1234"),
            "Ollama not running on 192.168.1.5:1234"
        );
    }

    struct PanicEngine;

    impl crate::mt_seq2seq::Seq2SeqMt for PanicEngine {
        fn translate_body(
            &self,
            _from: &str,
            _to: &str,
            _body: &str,
            _progress: &mut dyn FnMut(&str),
        ) -> Result<String, String> {
            panic!("seq2seq MT must not run when llm is false");
        }
    }

    struct MapEngine;

    impl crate::mt_seq2seq::Seq2SeqMt for MapEngine {
        fn translate_body(
            &self,
            _from: &str,
            _to: &str,
            body: &str,
            progress: &mut dyn FnMut(&str),
        ) -> Result<String, String> {
            progress("getting es-en…");
            Ok(match body {
                "hola" => "hello".into(),
                "gracias" => "thanks".into(),
                other => format!("MT:{other}"),
            })
        }
    }

    #[test]
    fn default_path_is_opus_not_lex() {
        assert!(super::default_use_opus());
        assert!(super::is_opus_unavailable(
            "Opus-MT files missing for es-en. Lex result is unchanged."
        ));
        assert!(super::is_opus_unavailable("Could not download es-en: timeout"));
        assert!(!super::is_opus_unavailable("Opus-MT repeated the source"));
    }

    #[tokio::test]
    async fn lex_default_does_not_need_ollama() {
        let out = super::translate("es", "en", "hola", false, None)
            .await
            .unwrap();
        assert_eq!(out.engine, "lex");
        assert_eq!(out.text, "Hello");
        assert_eq!(out.model, None);
    }

    #[tokio::test]
    async fn llm_false_never_calls_seq2seq_or_download() {
        let out = super::translate_with_engine("es", "en", "hola", false, &PanicEngine)
            .await
            .unwrap();
        assert_eq!(out.engine, "lex");
        assert_eq!(out.text, "Hello");
        assert_eq!(out.model, None);
    }

    #[tokio::test]
    async fn llm_true_uses_opus_engine_not_ollama() {
        let src = "Me: hola\n\nThem: gracias";
        let out = super::translate_with_engine("es", "en", src, true, &MapEngine)
            .await
            .unwrap();
        assert_eq!(out.engine, "opus");
        assert_eq!(out.model.as_deref(), Some("opus-mt-es-en"));
        assert_eq!(out.text, "Me: hello\n\nThem: thanks");
    }

    struct EchoEngine;

    impl crate::mt_seq2seq::Seq2SeqMt for EchoEngine {
        fn translate_body(
            &self,
            _from: &str,
            _to: &str,
            body: &str,
            _progress: &mut dyn FnMut(&str),
        ) -> Result<String, String> {
            Ok(body.to_string())
        }
    }

    struct MissingEngine;

    impl crate::mt_seq2seq::Seq2SeqMt for MissingEngine {
        fn translate_body(
            &self,
            _from: &str,
            _to: &str,
            _body: &str,
            _progress: &mut dyn FnMut(&str),
        ) -> Result<String, String> {
            Err("Opus-MT files missing for es-en. Lex result is unchanged.".into())
        }
    }

    #[tokio::test]
    async fn opus_missing_falls_back_to_lex_not_ollama() {
        let out = super::translate_with_engine("es", "en", "hola", true, &MissingEngine)
            .await
            .unwrap();
        assert_eq!(out.engine, "lex");
        assert_eq!(out.text, "Hello");
        assert!(
            out.warning.as_deref().unwrap_or("").contains("files missing"),
            "{:?}",
            out.warning
        );
        assert!(!out.warning.as_deref().unwrap_or("").contains("Ollama"));
    }

    #[tokio::test]
    async fn llm_echo_is_error_so_dest_stays() {
        let err = match super::translate_with_engine("es", "en", "hola", true, &EchoEngine).await {
            Ok(out) => panic!("echo must fail, got {}", out.text),
            Err(e) => e,
        };
        assert!(err.contains("repeated the source"), "{err}");
        assert!(!err.to_ascii_lowercase().contains("ollama"));
    }

    #[tokio::test]
    async fn lex_chat_labels_do_not_need_ollama() {
        let src = "Me: hola\n\nThem: Buenas tardes tiene paquete";
        let out = super::translate("es", "en", src, false, None)
            .await
            .unwrap();
        assert_eq!(out.engine, "lex");
        assert_eq!(out.model, None);
        assert_eq!(
            out.text,
            "Me: Hello\n\nThem: Good afternoon, you have a package"
        );
    }

    #[tokio::test]
    async fn same_language_returns_source() {
        let out = super::translate("es", "es", "Hola", false, None)
            .await
            .unwrap();
        assert_eq!(out.text, "Hola");
        assert_eq!(out.engine, "same");
        assert_eq!(out.model, None);
    }

    #[tokio::test]
    #[ignore = "needs cached Opus-MT weights"]
    async fn live_mexican_spanish_american_english() {
        let es_en = super::translate(
            "es",
            "en",
            "Hola, ¿cómo estás? Te mando un mensaje.",
            true,
            None,
        )
        .await
        .expect("es→en");
        assert!(
            !es_en.text.is_empty() && !es_en.text.to_lowercase().contains("hola"),
            "es→en echoed or empty: {}",
            es_en.text
        );

        let en_es = super::translate(
            "en",
            "es",
            "Can you send me a photo of your apartment? I'll call you from my cell phone.",
            true,
            None,
        )
        .await
        .expect("en→es");
        let low = en_es.text.to_lowercase();
        assert!(
            !en_es.text.is_empty() && !low.contains("apartment"),
            "en→es echoed or empty: {}",
            en_es.text
        );
        assert!(
            low.contains("departamento") || low.contains("celular") || low.contains("foto"),
            "en→es missing Mexican vocab: {}",
            en_es.text
        );
    }
}
