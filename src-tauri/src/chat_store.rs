//! Chat transcript on disk: %APPDATA%\lingua\chat.json.
//!
//! Written through a .tmp + rename so a crash mid-write cannot leave a truncated
//! transcript, and read leniently: a corrupt or partial file degrades to an empty
//! log rather than blocking the chat pane.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Keep the tail only. A transcript is a convenience, not an archive, and the
/// whole thing is replayed to the model on every turn.
pub const MAX_MSGS: usize = 200;

/// One persisted turn. `backend` records which engine that row actually went
/// through, so a thread the picker was switched inside still reads correctly on
/// restore. It is storage-only: the wire type is `ollama::ChatMsg`, which has no
/// such field, so serde drops it before either provider sees a request body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatRow {
    pub role: String,
    pub content: String,
    /// "ollama" | "dsf". Empty for rows written before this field existed.
    #[serde(default)]
    pub backend: String,
}

/// `engine` is the picker's position when the log was last saved. It is the
/// default for the next turn, not a claim about the rows above it -- per-row
/// truth lives in `ChatRow::backend`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatLog {
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub messages: Vec<ChatRow>,
}

pub fn path() -> PathBuf {
    crate::config::user_config_dir().join("chat.json")
}

pub fn trim(mut log: ChatLog) -> ChatLog {
    if log.messages.len() > MAX_MSGS {
        let drop = log.messages.len() - MAX_MSGS;
        log.messages.drain(..drop);
    }
    log
}

pub fn load_from(p: &Path) -> ChatLog {
    let Ok(raw) = fs::read_to_string(p) else {
        return ChatLog::default();
    };
    trim(serde_json::from_str(&raw).unwrap_or_default())
}

pub fn save_to(p: &Path, log: &ChatLog) -> Result<(), String> {
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(log).map_err(|e| format!("serialize chat.json: {e}"))?;
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, &bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    if p.exists() {
        let _ = fs::remove_file(p);
    }
    fs::rename(&tmp, p).map_err(|e| format!("rename {}: {e}", p.display()))
}

pub fn load() -> ChatLog {
    let p = path();
    if p.is_file() {
        return load_from(&p);
    }
    let legacy = crate::config::pulse_config_dir().join("chat.json");
    if legacy.is_file() {
        let log = load_from(&legacy);
        let _ = save(&log);
        return log;
    }
    ChatLog::default()
}

pub fn save(log: &ChatLog) -> Result<(), String> {
    save_to(&path(), &trim(log.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> ChatRow {
        row(role, content, "")
    }

    fn row(role: &str, content: &str, backend: &str) -> ChatRow {
        ChatRow {
            role: role.into(),
            content: content.into(),
            backend: backend.into(),
        }
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pulse-chat-{tag}-{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        d
    }

    #[test]
    fn round_trips_through_a_file() {
        let p = tmp_dir("rt").join("chat.json");
        let log = ChatLog {
            engine: "dsf".into(),
            model: "DeepSeek-V4-Flash".into(),
            messages: vec![msg("user", "ping"), msg("assistant", "OK")],
        };
        save_to(&p, &log).unwrap();
        let back = load_from(&p);
        assert_eq!(back.engine, "dsf");
        assert_eq!(back.model, "DeepSeek-V4-Flash");
        assert_eq!(back.messages.len(), 2);
        assert_eq!(back.messages[1].content, "OK");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn missing_file_is_an_empty_log_not_an_error() {
        let p = tmp_dir("missing").join("nope.json");
        let _ = fs::remove_file(&p);
        assert!(load_from(&p).messages.is_empty());
    }

    #[test]
    fn corrupt_file_degrades_to_empty() {
        let p = tmp_dir("corrupt").join("chat.json");
        fs::write(&p, b"{not json").unwrap();
        assert!(load_from(&p).messages.is_empty());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn save_leaves_no_tmp_behind() {
        let p = tmp_dir("tmp").join("chat.json");
        save_to(&p, &ChatLog::default()).unwrap();
        assert!(!p.with_extension("json.tmp").exists(), "tmp file survived");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn overwriting_replaces_rather_than_appends() {
        let p = tmp_dir("over").join("chat.json");
        save_to(
            &p,
            &ChatLog {
                messages: vec![msg("user", "first")],
                ..Default::default()
            },
        )
        .unwrap();
        save_to(
            &p,
            &ChatLog {
                messages: vec![msg("user", "second")],
                ..Default::default()
            },
        )
        .unwrap();
        let back = load_from(&p);
        assert_eq!(back.messages.len(), 1);
        assert_eq!(back.messages[0].content, "second");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn trim_keeps_the_newest_messages() {
        let log = ChatLog {
            messages: (0..MAX_MSGS + 10)
                .map(|i| msg("user", &i.to_string()))
                .collect(),
            ..Default::default()
        };
        let t = trim(log);
        assert_eq!(t.messages.len(), MAX_MSGS);
        assert_eq!(t.messages[0].content, "10");
        assert_eq!(t.messages[MAX_MSGS - 1].content, (MAX_MSGS + 9).to_string());
    }

    #[test]
    fn path_is_under_the_lingua_config_dir() {
        let p = path();
        assert_eq!(p.file_name().unwrap(), "chat.json");
        assert_eq!(p.parent().unwrap(), crate::config::user_config_dir());
        assert!(p.to_string_lossy().replace('\\', "/").contains("/lingua/"));
    }

    #[test]
    fn backend_survives_the_round_trip() {
        let p = tmp_dir("backend").join("chat.json");
        let log = ChatLog {
            engine: "dsf".into(),
            model: "DeepSeek-V4-Flash".into(),
            messages: vec![row("user", "ping", "dsf"), row("assistant", "OK", "dsf")],
        };
        save_to(&p, &log).unwrap();
        let back = load_from(&p);
        assert_eq!(back.messages[0].backend, "dsf");
        assert_eq!(back.messages[1].backend, "dsf");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn a_thread_the_picker_was_switched_inside_keeps_both_backends() {
        let p = tmp_dir("mixed").join("chat.json");
        let log = ChatLog {
            // The picker ended on dsf, but the first pair went to ollama. Only
            // per-row backend can tell those apart after a restart.
            engine: "dsf".into(),
            model: "DeepSeek-V4-Flash".into(),
            messages: vec![
                row("user", "local?", "ollama"),
                row("assistant", "yes", "ollama"),
                row("user", "cloud?", "dsf"),
                row("assistant", "yes", "dsf"),
            ],
        };
        save_to(&p, &log).unwrap();
        let back = load_from(&p);
        let seen: Vec<&str> = back.messages.iter().map(|m| m.backend.as_str()).collect();
        assert_eq!(seen, ["ollama", "ollama", "dsf", "dsf"]);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn rows_written_before_backend_existed_still_load() {
        // The shape CC-P3 shipped first: role + content, no backend key.
        let p = tmp_dir("legacy").join("chat.json");
        fs::write(
            &p,
            br#"{"engine":"ollama","model":"llama3","messages":[{"role":"user","content":"hi"}]}"#,
        )
        .unwrap();
        let back = load_from(&p);
        assert_eq!(back.messages.len(), 1, "legacy log must not be discarded");
        assert_eq!(back.messages[0].content, "hi");
        assert_eq!(back.messages[0].backend, "", "unknown, not guessed");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn backend_is_written_as_a_column_on_every_row() {
        let json = serde_json::to_string(&ChatLog {
            engine: "dsf".into(),
            model: "m".into(),
            messages: vec![row("user", "x", "dsf")],
        })
        .unwrap();
        assert!(json.contains(r#""backend":"dsf""#), "{json}");
    }

    // --- CC-P3 restart gate -------------------------------------------------
    // Two separate cargo processes: `write` then `read`. Run them in order:
    //   cargo test --lib chat_store::tests::restart_gate_write -- --ignored --nocapture
    //   cargo test --lib chat_store::tests::restart_gate_read  -- --ignored --nocapture
    // They use the real %APPDATA%\pulse\chat.json, so the second process proves
    // the transcript survives a full process restart, not just an in-memory copy.

    pub(super) const GATE_MARK: &str = "CC-P3 restart marker";

    #[test]
    #[ignore = "restart gate: writes the real %APPDATA%/pulse/chat.json"]
    fn restart_gate_write() {
        let log = ChatLog {
            engine: "dsf".into(),
            model: "DeepSeek-V4-Flash".into(),
            messages: vec![
                row("user", GATE_MARK, "ollama"),
                row("assistant", "still here", "ollama"),
                row("user", "and after the switch", "dsf"),
                row("assistant", "still here too", "dsf"),
            ],
        };
        save(&log).unwrap();
        println!("wrote {} (pid {})", path().display(), std::process::id());
    }

    #[test]
    #[ignore = "restart gate: reads the real %APPDATA%/pulse/chat.json"]
    fn restart_gate_read() {
        let back = load();
        println!("read {} (pid {})", path().display(), std::process::id());
        println!(
            "engine={} model={} messages={}",
            back.engine,
            back.model,
            back.messages.len()
        );
        for m in &back.messages {
            println!("  [{}] {}: {}", m.backend, m.role, m.content);
        }
        assert_eq!(
            back.messages.first().map(|m| m.content.as_str()),
            Some(GATE_MARK)
        );
        assert_eq!(back.engine, "dsf");
        let seen: Vec<&str> = back.messages.iter().map(|m| m.backend.as_str()).collect();
        assert_eq!(
            seen,
            ["ollama", "ollama", "dsf", "dsf"],
            "per-row backend must survive the process restart"
        );
    }
}
