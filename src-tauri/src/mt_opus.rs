//! In-process Helsinki-NLP Opus-MT (Marian) via Candle. No Ollama.

use candle_core::{Device, DType, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::marian::{self, MTModel};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokenizers::Tokenizer;

const UA: &str = concat!(
    "pulse/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/Randy-L-Thomas/pulse)"
);
const MIN_WEIGHT_BYTES: u64 = 80 * 1024 * 1024;
const MIN_TOKENIZER_BYTES: u64 = 8 * 1024;

#[derive(Debug)]
struct Pair {
    id: &'static str,
    weight_url: &'static str,
    tokenizer_url: &'static str,
}

const ES_EN: Pair = Pair {
    id: "es-en",
    weight_url: "https://huggingface.co/Helsinki-NLP/opus-mt-es-en/resolve/refs%2Fpr%2F4/model.safetensors",
    tokenizer_url: "https://huggingface.co/Xenova/opus-mt-es-en/resolve/main/tokenizer.json",
};

const EN_ES: Pair = Pair {
    id: "en-es",
    weight_url: "https://huggingface.co/Helsinki-NLP/opus-mt-en-es/resolve/refs%2Fpr%2F4/model.safetensors",
    tokenizer_url: "https://huggingface.co/Xenova/opus-mt-en-es/resolve/main/tokenizer.json",
};

struct Loaded {
    cfg: marian::Config,
    model: Mutex<MTModel>,
    tokenizer: Tokenizer,
}

static ES_EN_MODEL: Mutex<Option<std::sync::Arc<Loaded>>> = Mutex::new(None);
static EN_ES_MODEL: Mutex<Option<std::sync::Arc<Loaded>>> = Mutex::new(None);
static HTTP: OnceLock<reqwest::blocking::Client> = OnceLock::new();

fn pair(from: &str, to: &str) -> Result<&'static Pair, String> {
    match (from, to) {
        ("es", "en") => Ok(&ES_EN),
        ("en", "es") => Ok(&EN_ES),
        _ => Err(format!("Opus-MT has no {from}→{to} model")),
    }
}

fn dir_has_opus(dir: &std::path::Path) -> bool {
    dir.join("opus-mt-es-en").is_dir() || dir.join("opus-mt-en-es").is_dir()
}

fn mt_root() -> PathBuf {
    if let Ok(p) = std::env::var("LINGUA_MT_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(p) = std::env::var("PULSE_MT_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let own = crate::config::user_config_dir().join("mt");
    if dir_has_opus(&own) {
        return own;
    }
    let pulse = crate::config::pulse_config_dir().join("mt");
    if dir_has_opus(&pulse) {
        return pulse;
    }
    own
}

fn pair_dir(p: &Pair) -> PathBuf {
    mt_root().join(format!("opus-mt-{}", p.id))
}

fn http() -> Result<&'static reqwest::blocking::Client, String> {
    if let Some(c) = HTTP.get() {
        return Ok(c);
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30 * 60))
        .connect_timeout(Duration::from_secs(20))
        .no_proxy()
        .user_agent(UA)
        .build()
        .map_err(|e| format!("Opus-MT HTTP client: {e}"))?;
    let _ = HTTP.set(client);
    HTTP.get().ok_or_else(|| "Opus-MT HTTP client".into())
}

fn file_ok(path: &Path, min_bytes: u64) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() >= min_bytes)
}

/// Xenova Marian JSON uses `"precompiled_charsmap": null`, which the Rust
/// tokenizers crate rejects. NFC is enough for chat.
fn sanitize_tokenizer_json(path: &Path) -> Result<(), String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("Could not read tokenizer {}: {e}", path.display()))?;
    let mut v: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("Opus-MT tokenizer JSON: {e}"))?;
    let mut changed = false;
    if let Some(norm) = v.get_mut("normalizer") {
        let bad = norm.get("type").and_then(|t| t.as_str()) == Some("Precompiled")
            && norm
                .get("precompiled_charsmap")
                .map(|x| x.is_null())
                .unwrap_or(false);
        if bad {
            *norm = serde_json::json!({ "type": "NFC" });
            changed = true;
        }
    }
    if changed {
        fs::write(path, serde_json::to_string(&v).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Could not patch tokenizer: {e}"))?;
    }
    Ok(())
}

fn download_file(
    url: &str,
    dest: &Path,
    min_bytes: u64,
    label: &str,
    progress: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if file_ok(dest, min_bytes) {
        return Ok(());
    }
    progress(&format!("getting {label}…"));
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    }
    let tmp = dest.with_extension("part");
    let mut resp = http()?
        .get(url)
        .send()
        .map_err(|e| format!("Could not download {label}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "Could not download {label} (HTTP {})",
            resp.status()
        ));
    }
    let mut file = fs::File::create(&tmp)
        .map_err(|e| format!("Could not write {}: {e}", tmp.display()))?;
    resp.copy_to(&mut file)
        .map_err(|e| format!("Could not download {label}: {e}"))?;
    drop(file);
    if !file_ok(&tmp, min_bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not download {label} (file too small)"));
    }
    fs::rename(&tmp, dest).map_err(|e| format!("Could not save {label}: {e}"))?;
    Ok(())
}

fn ensure_files(p: &Pair, progress: &mut dyn FnMut(&str)) -> Result<(PathBuf, PathBuf), String> {
    let dir = pair_dir(p);
    let weights = dir.join("model.safetensors");
    let tokenizer = dir.join("tokenizer.json");
    download_file(p.weight_url, &weights, MIN_WEIGHT_BYTES, p.id, progress)?;
    download_file(
        p.tokenizer_url,
        &tokenizer,
        MIN_TOKENIZER_BYTES,
        p.id,
        progress,
    )?;
    sanitize_tokenizer_json(&tokenizer)?;
    if !file_ok(&weights, MIN_WEIGHT_BYTES) || !file_ok(&tokenizer, MIN_TOKENIZER_BYTES) {
        return Err(format!(
            "Opus-MT files missing for {}. Lex result is unchanged.",
            p.id
        ));
    }
    Ok((weights, tokenizer))
}

fn slot(from: &str, to: &str) -> Result<&'static Mutex<Option<std::sync::Arc<Loaded>>>, String> {
    match (from, to) {
        ("es", "en") => Ok(&ES_EN_MODEL),
        ("en", "es") => Ok(&EN_ES_MODEL),
        _ => Err(format!("Opus-MT has no {from}→{to} model")),
    }
}

fn load_pair(p: &Pair, progress: &mut dyn FnMut(&str)) -> Result<std::sync::Arc<Loaded>, String> {
    let (weights, tokenizer_path) = ensure_files(p, progress)?;
    progress(&format!("loading {}…", p.id));
    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| format!("Opus-MT tokenizer {}: {e}", p.id))?;
    let device = Device::Cpu;
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[&weights], DType::F32, &device)
            .map_err(|e| format!("Opus-MT could not load {}: {e}", p.id))?
    };
    let cfg = marian::Config::opus_mt_en_es();
    let model =
        MTModel::new(&cfg, vb).map_err(|e| format!("Opus-MT could not init {}: {e}", p.id))?;
    Ok(std::sync::Arc::new(Loaded {
        cfg,
        model: Mutex::new(model),
        tokenizer,
    }))
}

fn loaded(from: &str, to: &str, progress: &mut dyn FnMut(&str)) -> Result<std::sync::Arc<Loaded>, String> {
    let p = pair(from, to)?;
    {
        let guard = slot(from, to)?.lock().map_err(|e| e.to_string())?;
        if let Some(hit) = guard.as_ref() {
            return Ok(hit.clone());
        }
    }
    let model = load_pair(p, progress)?;
    let mut guard = slot(from, to)?.lock().map_err(|e| e.to_string())?;
    *guard = Some(model.clone());
    Ok(model)
}

fn argmax_u32(logits: &Tensor) -> Result<u32, String> {
    let vals = logits.to_vec1::<f32>().map_err(|e| e.to_string())?;
    let mut best_i = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, v) in vals.iter().copied().enumerate() {
        if v > best_v {
            best_v = v;
            best_i = i;
        }
    }
    Ok(best_i as u32)
}

fn infer(loaded: &Loaded, text: &str) -> Result<String, String> {
    let mut model = loaded.model.lock().map_err(|e| e.to_string())?;
    model.reset_kv_cache();
    let encoding = loaded
        .tokenizer
        .encode(text, true)
        .map_err(|e| format!("Opus-MT encode: {e}"))?;
    let mut tokens = encoding.get_ids().to_vec();
    if tokens.last() != Some(&loaded.cfg.eos_token_id) {
        tokens.push(loaded.cfg.eos_token_id);
    }
    if tokens.len() > loaded.cfg.max_position_embeddings {
        tokens.truncate(loaded.cfg.max_position_embeddings);
        tokens[loaded.cfg.max_position_embeddings - 1] = loaded.cfg.eos_token_id;
    }
    let device = Device::Cpu;
    let input = Tensor::new(tokens.as_slice(), &device)
        .map_err(|e| e.to_string())?
        .unsqueeze(0)
        .map_err(|e| e.to_string())?;
    let encoder_xs = model
        .encoder()
        .forward(&input, 0)
        .map_err(|e| format!("Opus-MT encode: {e}"))?;

    let mut token_ids = vec![loaded.cfg.decoder_start_token_id];
    for index in 0..192 {
        let context_size = if index >= 1 { 1 } else { token_ids.len() };
        let start_pos = token_ids.len().saturating_sub(context_size);
        let input_ids = Tensor::new(&token_ids[start_pos..], &device)
            .map_err(|e| e.to_string())?
            .unsqueeze(0)
            .map_err(|e| e.to_string())?;
        let logits = model
            .decode(&input_ids, &encoder_xs, start_pos)
            .map_err(|e| format!("Opus-MT decode: {e}"))?;
        let logits = logits.squeeze(0).map_err(|e| e.to_string())?;
        let last = logits.dim(0).map_err(|e| e.to_string())? - 1;
        let logits = logits.get(last).map_err(|e| e.to_string())?;
        let token = argmax_u32(&logits)?;
        token_ids.push(token);
        if token == loaded.cfg.eos_token_id || token == loaded.cfg.forced_eos_token_id {
            break;
        }
    }
    model.reset_kv_cache();
    let to_decode: Vec<u32> = token_ids
        .into_iter()
        .skip(1)
        .filter(|t| {
            *t != loaded.cfg.eos_token_id
                && *t != loaded.cfg.forced_eos_token_id
                && *t != loaded.cfg.pad_token_id
        })
        .collect();
    let out = loaded
        .tokenizer
        .decode(&to_decode, true)
        .map_err(|e| format!("Opus-MT decode: {e}"))?;
    Ok(out.trim().to_string())
}

pub fn translate(
    from: &str,
    to: &str,
    src: &str,
    progress: &mut dyn FnMut(&str),
) -> Result<String, String> {
    crate::mt_seq2seq::run_seq2seq(&OpusEngine, from, to, src, progress)
}

struct OpusEngine;

impl crate::mt_seq2seq::Seq2SeqMt for OpusEngine {
    fn translate_body(
        &self,
        from: &str,
        to: &str,
        body: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let model = loaded(from, to, progress)?;
        infer(&model, body)
    }
}

#[cfg(test)]
mod tests {
    use super::{pair, sanitize_tokenizer_json, EN_ES, ES_EN};
    use std::fs;

    #[test]
    fn patches_null_precompiled_charsmap() {
        let dir = std::env::temp_dir().join(format!(
            "pulse-tok-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokenizer.json");
        fs::write(
            &path,
            r#"{"version":"1.0","normalizer":{"type":"Precompiled","precompiled_charsmap":null},"model":{"type":"Unigram","unk_id":0,"vocab":[]}}"#,
        )
        .unwrap();
        sanitize_tokenizer_json(&path).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains(r#""type":"NFC""#));
        assert!(!raw.contains("precompiled_charsmap"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn routes_pairs_to_helsinki_not_ollama() {
        let es = pair("es", "en").unwrap();
        assert_eq!(es.id, "es-en");
        assert!(es.weight_url.contains("Helsinki-NLP/opus-mt-es-en"));
        assert!(es.tokenizer_url.contains("Xenova/opus-mt-es-en"));
        assert!(!es.weight_url.contains("ollama"));
        assert!(!es.weight_url.contains("11434"));
        let en = pair("en", "es").unwrap();
        assert_eq!(en.id, "en-es");
        assert!(en.weight_url.contains("Helsinki-NLP/opus-mt-en-es"));
        assert_eq!(ES_EN.id, "es-en");
        assert_eq!(EN_ES.id, "en-es");
    }

    #[test]
    fn missing_pair_is_plain_error() {
        let err = pair("fr", "en").unwrap_err();
        assert!(err.contains("Opus-MT"));
        assert!(!err.contains("Ollama"));
    }
}
