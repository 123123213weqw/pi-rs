//! Session persistence — JSONL format compatible with pi 0.74.2's session-manager.
//!
//! First line: session header `{type:"session", version:3, id, timestamp, cwd}`.
//! Then one entry per line: `{type:"message", id, parentId, timestamp, message}`.
//! Entry ids are 8-char uuid prefixes; the file is `{ISO-ts with :. -> -}_{id}.jsonl`.

use crate::messages::Message;
use crate::util::{full_uuid, iso_now, now_ms, short_id};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CURRENT_SESSION_VERSION: u64 = 3;

pub struct Session {
    pub id: String,
    pub file: PathBuf,
    writer: std::fs::File,
    last_entry_id: Option<String>,
    #[allow(dead_code)]
    persist: bool,
}

fn default_session_dir(cwd: &Path) -> PathBuf {
    let cwd_str = cwd.to_string_lossy().replace('\\', "/");
    let safe = format!("--{}--", cwd_str.trim_start_matches('/').replace(['/', ':'], "-"));
    let agent_dir = std::env::var("PI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".pi").join("agent"));
    agent_dir.join("sessions").join(safe)
}

pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

impl Session {
    /// Create (or open, for --continue) a session. `dir=None` uses pi's default
    /// `{agentDir}/sessions/{--encoded-cwd--}` layout.
    pub fn create(dir: Option<&Path>, cwd: &Path, persist: bool) -> std::io::Result<Session> {
        let id = full_uuid();
        let timestamp = iso_now();
        if !persist {
            return Ok(Session {
                id,
                file: PathBuf::new(),
                writer: std::fs::File::create(std::env::temp_dir().join("pi-rs-ephemeral"))
                    .unwrap_or_else(|_| {
                        // /dev/null equivalent for ephemeral sessions
                        std::fs::File::create("/dev/null").expect("cannot open /dev/null")
                    }),
                last_entry_id: None,
                persist,
            });
        }
        let dir = match dir {
            Some(d) => d.to_path_buf(),
            None => default_session_dir(cwd),
        };
        std::fs::create_dir_all(&dir)?;
        let file_ts = timestamp.replace([':', '.'], "-");
        let file = dir.join(format!("{file_ts}_{id}.jsonl"));
        let mut writer = std::fs::File::create(&file)?;

        let header = json!({
            "type": "session",
            "version": CURRENT_SESSION_VERSION,
            "id": id,
            "timestamp": timestamp,
            "cwd": cwd.to_string_lossy(),
        });
        writeln!(writer, "{header}")?;
        writer.flush()?;

        Ok(Session { id, file, writer, last_entry_id: None, persist })
    }

    /// pi v1.4.0 stores the system prompt as a `role:"system"` message entry
    /// carrying named sections (see system_prompt::build_sections).
    pub fn append_system_message(&mut self, sections: serde_json::Value) {
        let id = short_id();
        let entry = json!({
            "type": "message",
            "id": id,
            "parentId": self.last_entry_id,
            "timestamp": iso_now(),
            "message": {
                "role": "system",
                "content": "",
                "sections": sections,
            },
        });
        self.append_raw(entry);
        self.last_entry_id = Some(id);
    }

    /// pi writes model_change + thinking_level_change right after the header.
    pub fn append_setup_entries(&mut self, provider: &str, model_id: &str, thinking_level: &str) {
        let mc = json!({
            "type": "model_change",
            "id": short_id(),
            "parentId": self.last_entry_id,
            "timestamp": iso_now(),
            "provider": provider,
            "modelId": model_id,
        });
        self.append_raw(mc.clone());
        self.last_entry_id = mc.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let tc = json!({
            "type": "thinking_level_change",
            "id": short_id(),
            "parentId": self.last_entry_id,
            "timestamp": iso_now(),
            "thinkingLevel": thinking_level,
        });
        self.append_raw(tc.clone());
        self.last_entry_id = tc.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
    }

    /// Append a message entry; returns the entry id.
    pub fn append_message(&mut self, message: &Message) -> String {
        let id = short_id();
        let entry = json!({
            "type": "message",
            "id": id,
            "parentId": self.last_entry_id,
            "timestamp": iso_from_ms_opt(message.timestamp()),
            "message": serde_json::to_value(message).unwrap_or(Value::Null),
        });
        self.append_raw(entry);
        self.last_entry_id = Some(id.clone());
        id
    }

    fn append_raw(&mut self, entry: Value) {
        if !self.persist {
            return;
        }
        let _ = writeln!(self.writer, "{entry}");
        let _ = self.writer.flush();
    }
}

fn iso_from_ms_opt(ms: u64) -> String {
    if ms == 0 {
        return iso_now();
    }
    crate::util::iso_from_ms(ms)
}

/// Most recently modified *.jsonl in a session dir (for --continue).
pub fn find_most_recent_session(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let mtime = entry.metadata().and_then(|m| m.modified()).ok()?;
            if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                best = Some((mtime, path));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Read all messages from a session file (for --continue/--session).
pub fn load_messages(path: &Path) -> std::io::Result<Vec<Message>> {
    let text = std::fs::read_to_string(path)?;
    let mut messages = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("message") {
            if let Some(m) = v.get("message") {
                if let Ok(msg) = serde_json::from_value::<Message>(m.clone()) {
                    messages.push(msg);
                }
            }
        }
    }
    Ok(messages)
}

/// Timestamp helper for message creation sites.
pub fn ts() -> u64 {
    now_ms()
}
