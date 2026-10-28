//! read tool — port of `core/tools/read.ts` (text path; image support via
//! base64 attachment like pi, auto-resize omitted for the headless core).

use super::truncate::{self, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES};
use serde_json::{json, Value};
use std::path::Path;

pub const DESCRIPTION: &str = "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.";

pub static PARAMETERS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| json!({
    "type": "object",
    "required": ["path"],
        "properties": {
        "path": { "type": "string", "description": "Path to the file to read (relative or absolute)" },
        "offset": { "type": "number", "description": "Line number to start reading from (1-indexed)" },
        "limit": { "type": "number", "description": "Maximum number of lines to read" }
    }
}));

pub fn resolve_path(p: &str, cwd: &Path) -> std::path::PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

pub fn run(path: &str, offset: Option<f64>, limit: Option<f64>, cwd: &Path) -> Result<String, String> {
    let absolute = resolve_path(path, cwd);

    // Image passthrough as base64 attachment (pi: image content block).
    if let Some(mime) = detect_image_mime(&absolute) {
        let data = std::fs::read(&absolute).map_err(|e| format!("{e}"))?;
        use base64_lite::encode;
        return Ok(format!(
            "{{\"__image__\":{{\"mime\":\"{mime}\",\"data\":\"{}\"}}}}",
            encode(&data)
        ));
    }

    let bytes = std::fs::read(&absolute)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("File not found: {path}"),
            std::io::ErrorKind::PermissionDenied => format!("Permission denied: {path}"),
            _ => format!("Could not read file {path}: {e}"),
        })?;
    let text_content = String::from_utf8_lossy(&bytes).into_owned();

    let all_lines: Vec<&str> = text_content.split('\n').collect();
    let total_file_lines = all_lines.len();

    let start_line = offset.map(|o| (o as usize).saturating_sub(1)).unwrap_or(0);
    let start_line_display = start_line + 1;
    if start_line >= all_lines.len() {
        return Err(format!(
            "Offset {} is beyond end of file ({total_file_lines} lines total)",
            offset.map(|o| o.to_string()).unwrap_or_else(|| "0".into())
        ));
    }

    let selected: String;
    let mut user_limited: Option<usize> = None;
    match limit {
        Some(l) => {
            let end = (start_line + l as usize).min(all_lines.len());
            selected = all_lines[start_line..end].join("\n");
            user_limited = Some(end - start_line);
        }
        None => {
            selected = all_lines[start_line..].join("\n");
        }
    }

    let t = truncate::truncate_head(&selected, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES);
    let output_text = if t.first_line_exceeds_limit {
        let first_size = all_lines[start_line].len();
        format!(
            "[Line {start_line_display} is {}, exceeds {} limit. Use bash: sed -n '{start_line_display}p' {path} | head -c {DEFAULT_MAX_BYTES}]",
            crate::util::format_size(first_size),
            crate::util::format_size(DEFAULT_MAX_BYTES),
        )
    } else if t.truncated {
        let end_display = start_line_display + t.output_lines - 1;
        let next = end_display + 1;
        if t.truncated_by == Some("lines") {
            format!(
                "{}\n\n[Showing lines {start_line_display}-{end_display} of {total_file_lines}. Use offset={next} to continue.]",
                t.content
            )
        } else {
            format!(
                "{}\n\n[Showing lines {start_line_display}-{end_display} of {total_file_lines} ({} limit). Use offset={next} to continue.]",
                t.content,
                crate::util::format_size(DEFAULT_MAX_BYTES)
            )
        }
    } else if let Some(used) = user_limited {
        if start_line + used < all_lines.len() {
            let remaining = all_lines.len() - (start_line + used);
            let next = start_line + used + 1;
            format!("{}\n\n[{remaining} more lines in file. Use offset={next} to continue.]", t.content)
        } else {
            t.content.clone()
        }
    } else {
        t.content.clone()
    };

    Ok(output_text)
}

fn detect_image_mime(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Minimal base64 encoder (avoids a crate dependency).
pub mod base64_lite {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            out.push(TABLE[(n >> 18) as usize & 63] as char);
            out.push(TABLE[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
            out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
        }
        out
    }
}
