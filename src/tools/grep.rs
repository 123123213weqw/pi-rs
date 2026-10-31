//! grep tool — port of `core/tools/grep.ts`. Backends: external ripgrep with
//! pi's exact arg set (`--json --line-number --color=never --hidden`), output
//! `path:line: text`, 500-char line truncation, `[N matches limit reached…]`.

use super::truncate::{self, truncate_line, DEFAULT_MAX_BYTES, GREP_MAX_LINE_LENGTH};
use serde_json::{json, Value};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub const DEFAULT_LIMIT: usize = 100;

pub const DESCRIPTION: &str = "Search file contents for a pattern. Returns matching lines with file paths and line numbers. Respects .gitignore. Output is truncated to 100 matches or 50KB (whichever is hit first). Long lines are truncated to 500 chars.";

pub static PARAMETERS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| json!({
    "type": "object",
    "required": ["pattern"],
        "properties": {
        "pattern": { "type": "string", "description": "Search pattern (regex or literal string)" },
        "path": { "type": "string", "description": "Directory or file to search (default: current directory)" },
        "glob": { "type": "string", "description": "Filter files by glob pattern, e.g. '*.ts' or '**/*.spec.ts'" },
        "ignoreCase": { "type": "boolean", "description": "Case-insensitive search (default: false)" },
        "literal": { "type": "boolean", "description": "Treat pattern as literal string instead of regex (default: false)" },
        "context": { "type": "number", "description": "Number of lines to show before and after each match (default: 0)" },
        "limit": { "type": "number", "description": "Maximum number of matches to return (default: 100)" }
    }
}));

struct RgMatch {
    file: PathBuf,
    line_no: usize,
    line_text: Option<String>,
}

fn find_rg() -> Option<PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let p = Path::new(dir).join("rg");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn run(
    pattern: &str,
    path: Option<&str>,
    glob: Option<&str>,
    ignore_case: bool,
    literal: bool,
    context: usize,
    limit: Option<usize>,
    cwd: &Path,
) -> Result<String, String> {
    let rg = find_rg().ok_or_else(|| "ripgrep (rg) is not available and could not be downloaded".to_string())?;

    let search = super::read::resolve_path(path.unwrap_or("."), cwd);
    let is_dir = search.is_dir();
    if !search.exists() {
        return Err(format!("Path not found: {}", search.display()));
    }

    let effective_limit = limit.unwrap_or(DEFAULT_LIMIT).max(1);
    let context_v = context;

    let mut args: Vec<String> = vec![
        "--json".into(),
        "--line-number".into(),
        "--color=never".into(),
        "--hidden".into(),
    ];
    if ignore_case {
        args.push("--ignore-case".into());
    }
    if literal {
        args.push("--fixed-strings".into());
    }
    if let Some(g) = glob {
        args.push("--glob".into());
        args.push(g.into());
    }
    args.push("--".into());
    args.push(pattern.into());
    args.push(search.display().to_string());

    let child = std::process::Command::new(&rg)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run ripgrep: {e}"))?;

    let mut stdout = child.stdout.expect("stdout");
    let mut stderr_bytes = Vec::new();
    {
        let mut se = child.stderr.expect("stderr");
        let t = std::thread::spawn(move || {
            let _ = std::io::Read::read_to_end(&mut se, &mut stderr_bytes);
        });
        t.join().ok();
    }

    let reader = std::io::BufReader::new(stdout);
    let mut matches: Vec<RgMatch> = Vec::new();
    let mut match_count = 0usize;
    let mut match_limit_reached = false;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() || match_count >= effective_limit {
            if match_count >= effective_limit {
                break;
            }
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("match") {
            match_count += 1;
            let file = v
                .pointer("/data/path/text")
                .and_then(|x| x.as_str())
                .unwrap_or_default();
            let line_no = v
                .pointer("/data/line_number")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as usize;
            let line_text = v
                .pointer("/data/lines/text")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            matches.push(RgMatch {
                file: PathBuf::from(file),
                line_no,
                line_text,
            });
            if match_count >= effective_limit {
                match_limit_reached = true;
                break;
            }
        }
    }

    let format_path = |p: &Path| -> String {
        if is_dir {
            if let Ok(rel) = p.strip_prefix(&search) {
                return rel.display().to_string().replace('\\', "/");
            }
        }
        p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    };

    if matches.is_empty() {
        return Ok("No matches found".into());
    }

    let mut output_lines: Vec<String> = Vec::new();
    let mut lines_truncated = false;

    let file_lines = |p: &Path| -> Vec<String> {
        // Upstream (post-v1.4.0) only normalizes CRLF; a lone CR is no longer a line break.
        std::fs::read_to_string(p)
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .split('\n')
            .map(|s| s.to_string())
            .collect()
    };

    for m in &matches {
        let rel = format_path(&m.file);
        if context_v == 0 {
            if let Some(text) = &m.line_text {
                let sanitized = text.replace("\r\n", "\n").replace('\r', "");
                let sanitized = sanitized.strip_suffix('\n').unwrap_or(&sanitized);
                let (t, was) = truncate_line(sanitized, GREP_MAX_LINE_LENGTH);
                if was {
                    lines_truncated = true;
                }
                output_lines.push(format!("{rel}:{}: {t}", m.line_no));
                continue;
            }
        }
        // context block
        let lines = file_lines(&m.file);
        if lines.is_empty() {
            output_lines.push(format!("{rel}:{}: (unable to read file)", m.line_no));
            continue;
        }
        let start = if context_v > 0 { m.line_no.saturating_sub(context_v).max(1) } else { m.line_no };
        let end = if context_v > 0 {
            (m.line_no + context_v).min(lines.len())
        } else {
            m.line_no
        };
        for cur in start..=end {
            let text = lines.get(cur - 1).map(|s| s.replace('\r', "")).unwrap_or_default();
            let (t, was) = truncate_line(&text, GREP_MAX_LINE_LENGTH);
            if was {
                lines_truncated = true;
            }
            if cur == m.line_no {
                output_lines.push(format!("{rel}:{cur}: {t}"));
            } else {
                output_lines.push(format!("{rel}-{cur}- {t}"));
            }
        }
    }

    let raw = output_lines.join("\n");
    // No line cap here: the match limit already bounds rows (pi: usize::MAX lines).
    let t = truncate::truncate_head(&raw, usize::MAX, DEFAULT_MAX_BYTES);
    let mut output = t.content.clone();
    let mut notices: Vec<String> = Vec::new();
    if match_limit_reached {
        notices.push(format!(
            "{effective_limit} matches limit reached. Use limit={} for more, or refine pattern",
            effective_limit * 2
        ));
    }
    if t.truncated {
        notices.push(format!("{} limit reached", crate::util::format_size(DEFAULT_MAX_BYTES)));
    }
    if lines_truncated {
        notices.push(format!(
            "Some lines truncated to {GREP_MAX_LINE_LENGTH} chars. Use read tool to see full lines"
        ));
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join(". ")));
    }
    Ok(output)
}
