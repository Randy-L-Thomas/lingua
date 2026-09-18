use serde::Deserialize;

#[derive(Deserialize)]
struct Tags {
    models: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    name: String,
}

pub fn host_label(base: &str) -> String {
    let s = base.trim().trim_end_matches('/');
    s.strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s)
        .to_string()
}

pub fn down_message(base: &str) -> String {
    format!("Ollama not running on {}", host_label(base))
}

fn is_connect_fail(msg: &str) -> bool {
    let l = msg.to_ascii_lowercase();
    l.contains("connection refused")
        || l.contains("actively refused")
        || l.contains("tcp connect")
        || l.contains("error sending request")
        || l.contains("timed out")
        || l.contains("timeout")
        || l.contains("connect error")
        || l.contains("error waiting for response")
}

pub fn map_connect_err(err: impl std::fmt::Display, base: &str) -> String {
    let msg = err.to_string();
    if is_connect_fail(&msg) {
        return down_message(base);
    }
    if msg.starts_with("Ollama") {
        msg
    } else {
        format!("Ollama: {msg}")
    }
}

pub async fn list_models(client: &reqwest::Client, base: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    let res = client.get(url).send().await.map_err(|e| map_connect_err(e, base))?;
    if !res.status().is_success() {
        return Err(format!("Ollama HTTP {}", res.status()));
    }
    let tags: Tags = res.json().await.map_err(|e| e.to_string())?;
    Ok(tags.models.into_iter().map(|m| m.name).collect())
}

/// The wire shape for both providers: exactly `role` + `content`.
///
/// The UI's rows also carry a `backend` tag (see `chat_store::ChatRow`), which
/// is storage-only. Deserializing into this struct is what strips it: serde
/// ignores unknown keys, so the tag is gone before a request body is built and
/// neither Ollama nor Foundry is ever sent a field it did not ask for.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMsg {
    pub role: String,
    pub content: String,
}

pub async fn chat(
    client: &reqwest::Client,
    base: &str,
    model: &str,
    messages: Vec<ChatMsg>,
) -> Result<String, String> {
    if model.trim().is_empty() {
        return Err("pick a model".into());
    }
    let url = format!("{}/api/chat", base.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "options": { "temperature": 0.7, "num_predict": 1024 }
    });
    let res = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| map_connect_err(e, base))?;
    if !res.status().is_success() {
        return Err(format!("Ollama HTTP {}", res.status()));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(v.pointer("/message/content")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::{host_label, map_connect_err, ChatMsg};

    #[test]
    fn the_backend_tag_is_stripped_before_the_wire() {
        let from_ui = r#"{"role":"user","content":"hi","backend":"dsf"}"#;
        let msg: ChatMsg = serde_json::from_str(from_ui).unwrap();
        let body = serde_json::to_string(&msg).unwrap();
        assert!(
            !body.contains("backend"),
            "storage-only field reached a provider body: {body}"
        );
        assert_eq!(body, r#"{"role":"user","content":"hi"}"#);
    }

    #[test]
    fn host_label_strips_scheme() {
        assert_eq!(host_label("http://127.0.0.1:11434"), "127.0.0.1:11434");
        assert_eq!(host_label("http://192.168.1.5:1234/"), "192.168.1.5:1234");
    }

    #[test]
    fn connect_fail_uses_configured_host() {
        assert_eq!(
            map_connect_err("tcp connect error: connection refused", "http://10.0.0.2:11435"),
            "Ollama not running on 10.0.0.2:11435"
        );
    }
}
