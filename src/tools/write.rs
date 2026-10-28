//! write tool — port of `core/tools/write.ts`. Creates parents, overwrites,
//! success message matches pi (`Successfully wrote N bytes to <path>`).

use serde_json::{json, Value};
use std::path::Path;

pub const DESCRIPTION: &str =
    "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.";

pub static PARAMETERS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| json!({
    "type": "object",
    "properties": {
        "path": { "type": "string", "description": "Path to the file to write (relative or absolute)" },
        "content": { "type": "string", "description": "Content to write to the file" }
    },
    "required": ["path", "content"]
}));

pub fn run(path: &str, content: &str, cwd: &Path) -> Result<String, String> {
    let p = Path::new(path);
    let absolute = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    if let Some(parent) = absolute.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Could not create directory {parent:?}: {e}"))?;
    }
    std::fs::write(&absolute, content).map_err(|e| format!("Could not write file {path}: {e}"))?;
    // pi uses JS string length (UTF-16 code units); for the tool result this is
    // character count for ASCII and matches UTF-16 length exactly.
    let len_units: usize = content.chars().map(|c| if (c as u32) > 0xFFFF { 2 } else { 1 }).sum();
    Ok(format!("Successfully wrote {len_units} bytes to {path}"))
}
