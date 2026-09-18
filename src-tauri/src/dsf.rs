//! DeepSeek-on-Foundry transport.
//!
//! Every outbound URL passes `guard` first. The allowlist is a single host and
//! is checked on the parsed URL, before any client is touched, so a wrong base
//! (api.deepseek.com, a look-alike suffix, a userinfo trick) fails as a plain
//! Err and never opens a socket.

use reqwest::Url;

/// The only host this module may talk to.
pub const ALLOWED_HOST: &str = "l2cybersec-foundry-eus.services.ai.azure.com";

pub const DEFAULT_BASE: &str = "https://l2cybersec-foundry-eus.services.ai.azure.com";
pub const DEFAULT_DEPLOYMENT: &str = "DeepSeek-V4-Flash";
const CHAT_PATH: &str = "/openai/v1/chat/completions";

pub const KEY_ENV: &str = "AZURE_FOUNDRY_API_KEY";
pub const BASE_ENV: &str = "AZURE_FOUNDRY_ENDPOINT";
pub const DEPLOYMENT_ENV: &str = "AZURE_FOUNDRY_DEPLOYMENT";

pub fn blocked_msg(host: &str) -> String {
    format!("DSF blocked: host {host} is not {ALLOWED_HOST}")
}

/// Parse `raw` and allow it only if it is https, on `ALLOWED_HOST`, default port.
/// Pure: performs no DNS and no I/O.
pub fn guard(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|e| format!("DSF blocked: bad url ({e})"))?;
    if url.scheme() != "https" {
        return Err(format!("DSF blocked: scheme {} is not https", url.scheme()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(blocked_msg(url.host_str().unwrap_or("?")));
    }
    let host = url.host_str().unwrap_or_default();
    if !host.eq_ignore_ascii_case(ALLOWED_HOST) {
        return Err(blocked_msg(host));
    }
    if let Some(port) = url.port() {
        if port != 443 {
            return Err(format!("DSF blocked: port {port} is not 443"));
        }
    }
    Ok(url)
}

/// Build the guarded chat URL for a base. Pure: no I/O.
pub fn chat_url(base: &str) -> Result<Url, String> {
    let base = base.trim().trim_end_matches('/');
    guard(&format!("{base}{CHAT_PATH}"))
}

pub fn base_from_env() -> String {
    std::env::var(BASE_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

pub fn deployment_from_env() -> String {
    std::env::var(DEPLOYMENT_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_DEPLOYMENT.to_string())
}

pub fn key_from_env() -> Result<String, String> {
    let k = std::env::var(KEY_ENV).unwrap_or_default().trim().to_string();
    if k.is_empty() {
        return Err(format!("DSF: {KEY_ENV} is not set"));
    }
    Ok(k)
}

fn body(model: &str, messages: &[crate::ollama::ChatMsg]) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "temperature": 0.7,
        "max_tokens": 1024
    })
}

/// Raw POST. Returns (http status, body text) without interpreting either.
pub async fn post_chat(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    model: &str,
    messages: &[crate::ollama::ChatMsg],
) -> Result<(u16, String), String> {
    let url = chat_url(base)?;
    let res = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("api-key", key)
        .json(&body(model, messages))
        .send()
        .await
        .map_err(|e| format!("DSF: {e}"))?;
    let status = res.status().as_u16();
    let text = res.text().await.map_err(|e| format!("DSF: {e}"))?;
    Ok((status, text))
}

pub fn parse_reply(status: u16, text: &str) -> Result<String, String> {
    if !(200..300).contains(&status) {
        let head: String = text.chars().take(160).collect();
        return Err(format!("DSF HTTP {status}: {head}"));
    }
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("DSF: bad json ({e})"))?;
    Ok(v.pointer("/choices/0/message/content")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string())
}

/// Shortest possible round trip. Reports the raw HTTP status and the head of the
/// body so a failure is legible without guessing.
pub async fn ping(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    model: &str,
) -> Result<String, String> {
    let probe = vec![crate::ollama::ChatMsg {
        role: "user".into(),
        content: "ping".into(),
    }];
    let (status, text) = post_chat(client, base, key, model, &probe).await?;
    Ok(format_ping(status, &text))
}

pub const PING_HEAD: usize = 80;

pub fn format_ping(status: u16, text: &str) -> String {
    let head: String = text.chars().take(PING_HEAD).collect();
    format!("HTTP {status} · {head}")
}

pub async fn chat(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    model: &str,
    messages: Vec<crate::ollama::ChatMsg>,
) -> Result<String, String> {
    if model.trim().is_empty() {
        return Err("DSF: pick a deployment".into());
    }
    let (status, text) = post_chat(client, base, key, model, &messages).await?;
    parse_reply(status, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ollama::ChatMsg;

    fn msg() -> Vec<ChatMsg> {
        vec![ChatMsg {
            role: "user".into(),
            content: "ping".into(),
        }]
    }

    #[test]
    fn deepseek_direct_is_blocked() {
        let err = chat_url("https://api.deepseek.com").unwrap_err();
        assert_eq!(err, blocked_msg("api.deepseek.com"));
    }

    #[tokio::test]
    async fn deepseek_direct_throws_before_any_http() {
        // A client that cannot complete a request: a 1ns timeout makes any real
        // socket attempt fail with a transport error, so only the allowlist can
        // produce the blocked-host message asserted below.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_nanos(1))
            .connect_timeout(std::time::Duration::from_nanos(1))
            .no_proxy()
            .build()
            .unwrap();
        let err = chat(&client, "https://api.deepseek.com", "k", "m", msg())
            .await
            .unwrap_err();
        assert_eq!(
            err,
            blocked_msg("api.deepseek.com"),
            "must fail in the allowlist, not in the transport"
        );
    }

    #[test]
    fn allowed_host_builds_the_chat_path() {
        let u = chat_url(DEFAULT_BASE).unwrap();
        assert_eq!(u.host_str(), Some(ALLOWED_HOST));
        assert_eq!(u.path(), CHAT_PATH);
        assert_eq!(u.scheme(), "https");
    }

    #[test]
    fn trailing_slash_does_not_double_up() {
        let u = chat_url("https://l2cybersec-foundry-eus.services.ai.azure.com/").unwrap();
        assert_eq!(u.path(), CHAT_PATH);
    }

    #[test]
    fn host_match_is_case_insensitive() {
        assert!(chat_url("https://L2CyberSec-Foundry-EUS.Services.AI.Azure.com").is_ok());
    }

    #[test]
    fn look_alike_suffix_is_blocked() {
        let host = "l2cybersec-foundry-eus.services.ai.azure.com.evil.test";
        assert_eq!(
            chat_url(&format!("https://{host}")).unwrap_err(),
            blocked_msg(host)
        );
    }

    #[test]
    fn userinfo_trick_is_blocked() {
        let raw = "https://l2cybersec-foundry-eus.services.ai.azure.com@api.deepseek.com/x";
        assert_eq!(guard(raw).unwrap_err(), blocked_msg("api.deepseek.com"));
    }

    #[test]
    fn sibling_foundry_hosts_are_blocked() {
        for host in [
            "l2cybersec-foundry-eus.cognitiveservices.azure.com",
            "l2cybersec-foundry-eus.openai.azure.com",
        ] {
            assert_eq!(
                chat_url(&format!("https://{host}")).unwrap_err(),
                blocked_msg(host)
            );
        }
    }

    #[test]
    fn plaintext_http_is_blocked() {
        let err = chat_url("http://l2cybersec-foundry-eus.services.ai.azure.com").unwrap_err();
        assert_eq!(err, "DSF blocked: scheme http is not https");
    }

    #[test]
    fn odd_port_is_blocked() {
        let err =
            chat_url("https://l2cybersec-foundry-eus.services.ai.azure.com:8443").unwrap_err();
        assert_eq!(err, "DSF blocked: port 8443 is not 443");
    }

    #[test]
    fn non_2xx_surfaces_status_and_head() {
        let err = parse_reply(401, r#"{"error":"PermissionDenied"}"#).unwrap_err();
        assert!(err.starts_with("DSF HTTP 401: "), "{err}");
    }

    #[test]
    fn ping_reports_status_and_first_80_chars() {
        let body = "x".repeat(500);
        let line = format_ping(200, &body);
        assert_eq!(line, format!("HTTP 200 · {}", "x".repeat(PING_HEAD)));
    }

    #[test]
    fn ping_on_a_blocked_host_never_reaches_the_wire() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let err = rt
            .block_on(ping(&client, "https://api.deepseek.com", "k", "m"))
            .unwrap_err();
        assert_eq!(err, blocked_msg("api.deepseek.com"));
    }

    /// Live gate for CC-P2. Needs AZURE_FOUNDRY_API_KEY; run explicitly:
    ///   cargo test --lib dsf::tests::live_ping -- --ignored --nocapture
    #[test]
    #[ignore = "live: hits Azure AI Foundry"]
    fn live_ping() {
        let key = key_from_env().expect("AZURE_FOUNDRY_API_KEY");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .no_proxy()
            .build()
            .unwrap();
        let line = rt
            .block_on(ping(
                &client,
                &base_from_env(),
                &key,
                &deployment_from_env(),
            ))
            .expect("ping");
        println!("{line}");
        assert!(line.starts_with("HTTP 200 · "), "{line}");
    }

    #[test]
    fn reply_reads_openai_shape() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":" OK "}}]}"#;
        assert_eq!(parse_reply(200, raw).unwrap(), "OK");
    }
}
