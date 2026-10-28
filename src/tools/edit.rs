//! edit tool — port of `core/tools/edit.ts`. Multiple disjoint exact-text
//! replacements per call; BOM/CRLF aware; error messages match pi verbatim.

use super::edit_diff::{apply_edits_to_file_content, Edit};
use serde_json::{json, Value};
use std::path::Path;

pub const DESCRIPTION: &str = "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.";

pub static PARAMETERS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| json!({
    "type": "object",
    "properties": {
        "path": { "type": "string", "description": "Path to the file to edit (relative or absolute)" },
        "edits": {
            "type": "array",
            "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead.",
            "items": {
                "type": "object",
                "properties": {
                    "oldText": { "type": "string", "description": "Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call." },
                    "newText": { "type": "string", "description": "Replacement text for this targeted edit." }
                },
                "required": ["oldText", "newText"],
                "additionalProperties": false
            }
        }
    },
    "required": ["path", "edits"],
    "additionalProperties": false
}));

pub fn run(path: &str, edits: &[Edit], cwd: &Path) -> Result<String, String> {
    if path.trim().is_empty() {
        return Err("path must not be empty".into());
    }
    if edits.is_empty() {
        return Err(format!("edits[] must not be empty in {path}"));
    }
    let p = Path::new(path);
    let absolute = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };

    if !absolute.is_file() {
        return Err(format!(
            "Could not edit file: {path}. Error code: ENOENT."
        ));
    }
    let raw = std::fs::read_to_string(&absolute)
        .map_err(|e| format!("Could not edit file: {path}. {e}."))?;

    let final_content = apply_edits_to_file_content(&raw, edits, path)?;
    std::fs::write(&absolute, final_content).map_err(|e| format!("Could not write file {path}: {e}"))?;

    Ok(format!("Successfully replaced {} block(s) in {path}.", edits.len()))
}
