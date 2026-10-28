//! ls tool — port of `core/tools/ls.ts`. Case-insensitive alphabetical sort,
//! `/` suffix for dirs, dotfiles included, 500-entry default limit.

use super::truncate::{self, DEFAULT_MAX_BYTES};
use serde_json::{json, Value};
use std::path::Path;

pub const DEFAULT_LIMIT: usize = 500;

pub const DESCRIPTION: &str = "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. Output is truncated to 500 entries or 50KB (whichever is hit first).";

pub static PARAMETERS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| json!({
    "type": "object",
    "properties": {
        "path": { "type": "string", "description": "Directory to list (default: current directory)" },
        "limit": { "type": "number", "description": "Maximum number of entries to return (default: 500)" }
    }
}));

pub fn run(path: Option<&str>, limit: Option<usize>, cwd: &Path) -> Result<String, String> {
    let dir = super::read::resolve_path(path.unwrap_or("."), cwd);
    if !dir.exists() {
        return Err(format!("Path not found: {}", dir.display()));
    }
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", dir.display()));
    }
    let effective_limit = limit.unwrap_or(DEFAULT_LIMIT);

    let mut entries: Vec<String> = Vec::new();
    let rd = std::fs::read_dir(&dir).map_err(|e| format!("Cannot read directory: {e}"))?;
    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        entries.push(entry.file_name().to_string_lossy().into_owned());
    }
    entries.sort_by_key(|a| a.to_lowercase());

    let mut results: Vec<String> = Vec::new();
    let mut entry_limit_reached = false;
    for entry in entries {
        if results.len() >= effective_limit {
            entry_limit_reached = true;
            break;
        }
        let full = dir.join(&entry);
        match std::fs::metadata(&full) {
            Ok(m) if m.is_dir() => results.push(format!("{entry}/")),
            Ok(_) => results.push(entry),
            Err(_) => continue, // skip entries we cannot stat
        }
    }

    if results.is_empty() {
        return Ok("(empty directory)".into());
    }

    let raw = results.join("\n");
    let t = truncate::truncate_head(&raw, usize::MAX, DEFAULT_MAX_BYTES);
    let mut output = t.content.clone();
    let mut notices: Vec<String> = Vec::new();
    if entry_limit_reached {
        notices.push(format!(
            "{effective_limit} entries limit reached. Use limit={} for more",
            effective_limit * 2
        ));
    }
    if t.truncated {
        notices.push(format!("{} limit reached", crate::util::format_size(DEFAULT_MAX_BYTES)));
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join(". ")));
    }
    Ok(output)
}
