//! Port of `core/tools/truncate.ts` — line/byte truncation with exact
//! pi semantics (2000 lines / 50KB, head & tail, UTF-8 safe).

pub const DEFAULT_MAX_LINES: usize = 2000;
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024;
pub const GREP_MAX_LINE_LENGTH: usize = 500;

#[derive(Debug, Clone, Default)]
pub struct TruncationResult {
    pub content: String,
    pub truncated: bool,
    pub truncated_by: Option<&'static str>, // "lines" | "bytes"
    pub total_lines: usize,
    pub total_bytes: usize,
    pub output_lines: usize,
    pub output_bytes: usize,
    pub last_line_partial: bool,
    pub first_line_exceeds_limit: bool,
    pub max_lines: usize,
    pub max_bytes: usize,
}

pub fn truncate_line(line: &str, max_chars: usize) -> (String, bool) {
    let count = line.chars().count();
    if count <= max_chars {
        return (line.to_string(), false);
    }
    let head: String = line.chars().take(max_chars).collect();
    (format!("{head}... [truncated]"), true)
}

fn no_truncation(content: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    TruncationResult {
        content: content.to_string(),
        truncated: false,
        truncated_by: None,
        total_lines: content.split('\n').count(),
        total_bytes: content.len(),
        output_lines: content.split('\n').count(),
        output_bytes: content.len(),
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

/// Keep the FIRST maxLines lines within maxBytes. Never returns partial lines.
pub fn truncate_head(content: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let total_bytes = content.len();
    let lines: Vec<&str> = content.split('\n').collect();
    let total_lines = lines.len();

    if total_lines <= max_lines && total_bytes <= max_bytes {
        return no_truncation(content, max_lines, max_bytes);
    }

    if lines[0].len() > max_bytes {
        return TruncationResult {
            content: String::new(),
            truncated: true,
            truncated_by: Some("bytes"),
            total_lines,
            total_bytes,
            output_lines: 0,
            output_bytes: 0,
            last_line_partial: false,
            first_line_exceeds_limit: true,
            max_lines,
            max_bytes,
        };
    }

    let mut out: Vec<&str> = Vec::new();
    let mut out_bytes = 0usize;
    let mut truncated_by = "lines";
    for (i, line) in lines.iter().enumerate() {
        if i >= max_lines {
            break;
        }
        let line_bytes = line.len() + if i > 0 { 1 } else { 0 };
        if out_bytes + line_bytes > max_bytes {
            truncated_by = "bytes";
            break;
        }
        out.push(line);
        out_bytes += line_bytes;
    }
    if out.len() >= max_lines && out_bytes <= max_bytes {
        truncated_by = "lines";
    }

    let output_content = out.join("\n");
    TruncationResult {
        output_bytes: output_content.len(),
        content: output_content,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        output_lines: out.len(),
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

/// Keep the LAST maxLines lines within maxBytes (bash semantics). The first
/// line may be partial (cut from the end of the original line, UTF-8 safe).
pub fn truncate_tail(content: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let total_bytes = content.len();
    let lines: Vec<&str> = content.split('\n').collect();
    let total_lines = lines.len();

    if total_lines <= max_lines && total_bytes <= max_bytes {
        return no_truncation(content, max_lines, max_bytes);
    }

    let mut out: Vec<String> = Vec::new();
    let mut out_bytes = 0usize;
    let mut truncated_by = "lines";
    let mut last_line_partial = false;

    for line in lines.iter().rev() {
        if out.len() >= max_lines {
            break;
        }
        let line_bytes = line.len() + if !out.is_empty() { 1 } else { 0 };
        if out_bytes + line_bytes > max_bytes {
            truncated_by = "bytes";
            if out.is_empty() {
                let cut = truncate_bytes_from_end(line, max_bytes);
                out_bytes = cut.len();
                out.insert(0, cut);
                last_line_partial = true;
            }
            break;
        }
        out.insert(0, (*line).to_string());
        out_bytes += line_bytes;
    }
    if out.len() >= max_lines && out_bytes <= max_bytes {
        truncated_by = "lines";
    }

    let output_content = out.join("\n");
    TruncationResult {
        output_bytes: output_content.len(),
        content: output_content,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        output_lines: out.len(),
        last_line_partial,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

fn truncate_bytes_from_end(s: &str, max_bytes: usize) -> String {
    let buf = s.as_bytes();
    if buf.len() <= max_bytes {
        return s.to_string();
    }
    let mut start = buf.len() - max_bytes;
    while start < buf.len() && (buf[start] & 0xc0) == 0x80 {
        start += 1;
    }
    String::from_utf8_lossy(&buf[start..]).into_owned()
}
