//! find tool — port of `core/tools/find.ts`. Backend: external fd with pi's
//! exact args (`--glob --color=never --hidden --no-require-git --max-results`,
//! `--full-path` + `**/` prefixing for path patterns); internal walker fallback
//! with pi's custom-ops ignore list (`node_modules`, `.git`) when fd is absent.

use super::truncate::{self, DEFAULT_MAX_BYTES};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub const DEFAULT_LIMIT: usize = 1000;

pub const DESCRIPTION: &str = "Search for files by glob pattern. Returns matching file paths relative to the search directory. Respects .gitignore. Output is truncated to 1000 results or 50KB (whichever is hit first).";

pub static PARAMETERS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| json!({
    "type": "object",
    "required": ["pattern"],
        "properties": {
        "pattern": { "type": "string", "description": "Glob pattern to match files, e.g. '*.ts', '**/*.json', or 'src/**/*.spec.ts'" },
        "path": { "type": "string", "description": "Directory to search in (default: current directory)" },
        "limit": { "type": "number", "description": "Maximum number of results (default: 1000)" }
    },
    "required": ["pattern"]
}));

fn find_fd() -> Option<PathBuf> {
    for name in ["fd", "fdfind"] {
        for dir in std::env::var("PATH").unwrap_or_default().split(':') {
            let p = Path::new(dir).join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Tiny glob matcher supporting `*`, `**`, `?`, `{a,b}` via translation to a
/// recursive matcher over path segments.
fn glob_match(pattern: &str, path: &str) -> bool {
    // Split into segments; `**` matches any number of segments.
    let pat: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    seg_match(&pat, &segs)
}

fn seg_match(pat: &[&str], segs: &[&str]) -> bool {
    if pat.is_empty() {
        return segs.is_empty();
    }
    if pat[0] == "**" {
        // `**` can match zero or more segments
        for skip in 0..=segs.len() {
            if seg_match(&pat[1..], &segs[skip..]) {
                return true;
            }
        }
        return false;
    }
    if segs.is_empty() {
        return false;
    }
    if glob_seg(pat[0], segs[0]) {
        seg_match(&pat[1..], &segs[1..])
    } else {
        false
    }
}

fn glob_seg(pattern: &str, s: &str) -> bool {
    // char-wise glob with * and ?
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = s.chars().collect();
    glob_chars(&p, &t)
}

fn glob_chars(p: &[char], t: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    match p[0] {
        '*' => {
            for skip in 0..=t.len() {
                if glob_chars(&p[1..], &t[skip..]) {
                    return true;
                }
            }
            false
        }
        '?' => !t.is_empty() && glob_chars(&p[1..], &t[1..]),
        c => !t.is_empty() && t[0] == c && glob_chars(&p[1..], &t[1..]),
    }
}

fn should_ignore(rel: &str) -> bool {
    // pi's ignore list for custom glob ops: node_modules and .git
    rel.split('/').any(|seg| seg == "node_modules" || seg == ".git")
}

fn walk(dir: &Path, root: &Path, pattern: &str, limit: usize, out: &mut Vec<String>) {
    if out.len() >= limit {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut items: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    items.sort_by_key(|e| e.file_name());
    for entry in items {
        let path = entry.path();
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.display().to_string().replace('\\', "/"),
            Err(_) => continue,
        };
        let is_dir = path.is_dir();
        let rel_check = if is_dir { format!("{rel}/") } else { rel.clone() };
        if should_ignore(&rel_check) {
            continue;
        }
        if !is_dir && glob_match(pattern, &rel) {
            out.push(rel);
            if out.len() >= limit {
                return;
            }
        }
        if is_dir {
            walk(&path, root, pattern, limit, out);
        }
    }
}

pub fn run(pattern: &str, path: Option<&str>, limit: Option<usize>, cwd: &Path) -> Result<String, String> {
    let search = super::read::resolve_path(path.unwrap_or("."), cwd);
    if !search.exists() {
        return Err(format!("Path not found: {}", search.display()));
    }
    let effective_limit = limit.unwrap_or(DEFAULT_LIMIT);

    let mut lines: Vec<String> = Vec::new();
    let mut used_fd = false;

    if let Some(fd) = find_fd() {
        let mut args: Vec<String> = vec![
            "--glob".into(),
            "--color=never".into(),
            "--hidden".into(),
            "--no-require-git".into(),
            "--max-results".into(),
            effective_limit.to_string(),
        ];
        let mut effective_pattern = pattern.to_string();
        if pattern.contains('/') {
            args.push("--full-path".into());
            if !pattern.starts_with('/') && !pattern.starts_with("**/") && pattern != "**" {
                effective_pattern = format!("**/{pattern}");
            }
        }
        args.push("--".into());
        args.push(effective_pattern);
        args.push(search.display().to_string());

        // Older fd (v8, e.g. Ubuntu's fd-find) lacks --no-require-git (added in
        // v9, which pi's ensureTool ships). Retry without it when rejected.
        let run_fd = |args: &[String]| -> Result<std::process::Output, String> {
            std::process::Command::new(&fd)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .map_err(|e| format!("Failed to run fd: {e}"))
        };
        let mut out = run_fd(&args)?;
        if !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("no-require-git") {
            let retry: Vec<String> = args
                .iter()
                .filter(|a| a.as_str() != "--no-require-git")
                .cloned()
                .collect();
            out = run_fd(&retry)?;
        }
        used_fd = true;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        for raw in stdout.lines() {
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() {
                continue;
            }
            let had_slash = line.ends_with('/') || line.ends_with('\\');
            let mut rel = if let Ok(r) = Path::new(line).strip_prefix(&search) {
                r.display().to_string().replace('\\', "/")
            } else {
                line.to_string()
            };
            if had_slash && !rel.ends_with('/') {
                rel.push('/');
            }
            lines.push(rel);
        }
    } else {
        walk(&search, &search, pattern, effective_limit, &mut lines);
    }

    if lines.is_empty() {
        return Ok("No files found matching pattern".into());
    }

    let limit_reached = lines.len() >= effective_limit;
    let raw = lines.join("\n");
    let t = truncate::truncate_head(&raw, usize::MAX, DEFAULT_MAX_BYTES);
    let mut output = t.content.clone();
    let mut notices: Vec<String> = Vec::new();
    if limit_reached {
        notices.push(format!(
            "{effective_limit} results limit reached. Use limit={} for more, or refine pattern",
            effective_limit * 2
        ));
    }
    if t.truncated {
        notices.push(format!("{} limit reached", crate::util::format_size(DEFAULT_MAX_BYTES)));
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join(". ")));
    }
    let _ = used_fd;
    Ok(output)
}
