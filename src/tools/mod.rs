//! Tool registry — the 7 built-in tools of pi 0.74.2 with identical names,
//! descriptions, JSON schemas, and execution semantics.
//! Default enabled set (pi's "coding tools"): read, bash, edit, write.
//! Read-only tools (grep, find, ls) are enabled via `--tools`.

pub mod bash;
pub mod edit;
pub mod edit_diff;
pub mod find;
pub mod grep;
pub mod ls;
pub mod read;
pub mod truncate;
pub mod write;

use serde_json::Value;
use std::path::Path;

pub const ALL_TOOLS: [&str; 7] = ["read", "bash", "edit", "write", "grep", "find", "ls"];
pub const DEFAULT_TOOLS: [&str; 4] = ["read", "bash", "edit", "write"];
pub const READ_ONLY_TOOLS: [&str; 4] = ["read", "grep", "find", "ls"];

/// One-line snippets shown in the system prompt (pi: ToolDef.promptSnippet).
pub fn snippet_for(name: &str) -> Option<&'static str> {
    match name {
        "read" => Some("Read file contents"),
        "bash" => Some("Execute bash commands (ls, grep, find, etc.)"),
        "edit" => Some("Make precise file edits with exact text replacement, including multiple disjoint edits in one call"),
        "write" => Some("Create or overwrite files"),
        "grep" => Some("Search file contents for patterns (respects .gitignore)"),
        "find" => Some("Find files by glob pattern (respects .gitignore)"),
        "ls" => Some("List directory contents"),
        _ => None,
    }
}

#[derive(Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

/// Per-tool guideline bullets merged into the system prompt (pi: promptGuidelines).
pub fn prompt_guidelines(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "read" => Some(&["Use read to examine files instead of cat or sed."]),
        "edit" => Some(&[
            "Use edit for precise changes (edits[].oldText must match exactly)",
            "When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls",
            "Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit.",
            "Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.",
        ]),
        "write" => Some(&["Use write only for new files or complete rewrites."]),
        _ => None,
    }
}

pub fn tool_definition(name: &str) -> Option<ToolDef> {
    match name {
        "read" => Some(ToolDef { name: "read", description: read::DESCRIPTION, parameters: read::PARAMETERS.clone() }),
        "bash" => Some(ToolDef { name: "bash", description: bash::DESCRIPTION, parameters: bash::PARAMETERS.clone() }),
        "edit" => Some(ToolDef { name: "edit", description: edit::DESCRIPTION, parameters: edit::PARAMETERS.clone() }),
        "write" => Some(ToolDef { name: "write", description: write::DESCRIPTION, parameters: write::PARAMETERS.clone() }),
        "grep" => Some(ToolDef { name: "grep", description: grep::DESCRIPTION, parameters: grep::PARAMETERS.clone() }),
        "find" => Some(ToolDef { name: "find", description: find::DESCRIPTION, parameters: find::PARAMETERS.clone() }),
        "ls" => Some(ToolDef { name: "ls", description: ls::DESCRIPTION, parameters: ls::PARAMETERS.clone() }),
        _ => None,
    }
}

/// Execute a tool call. Returns the tool result text (is_error signaled via Err).
pub fn execute_tool(name: &str, args: &Value, cwd: &Path) -> Result<String, String> {
    match name {
        "read" => {
            let path = arg_str(args, "path").ok_or("missing required argument: path")?;
            read::run(&path, arg_num(args, "offset"), arg_num(args, "limit"), cwd)
        }
        "bash" => {
            let command = arg_str(args, "command").ok_or("missing required argument: command")?;
            bash::run(&command, cwd, arg_num(args, "timeout"))
                .map(|o| o.text)
        }
        "write" => {
            let path = arg_str(args, "path").ok_or("missing required argument: path")?;
            let content = args.get("content").and_then(|c| c.as_str()).ok_or("missing required argument: content")?;
            write::run(&path, content, cwd)
        }
        "edit" => {
            let path = arg_str(args, "path").ok_or("missing required argument: path")?;
            let arr = args.get("edits").and_then(|e| e.as_array()).ok_or("missing required argument: edits")?;
            let mut edits = Vec::new();
            for (i, e) in arr.iter().enumerate() {
                let old = e.get("oldText").and_then(|x| x.as_str()).unwrap_or("");
                let new = e.get("newText").and_then(|x| x.as_str()).unwrap_or("");
                edits.push(edit_diff::Edit { old_text: old.to_string(), new_text: new.to_string() });
                if old.is_empty() {
                    let total = arr.len();
                    return Err(if total == 1 {
                        format!("oldText must not be empty in {path}.")
                    } else {
                        format!("edits[{i}].oldText must not be empty in {path}.")
                    });
                }
            }
            edit::run(&path, &edits, cwd)
        }
        "grep" => {
            let pattern = arg_str(args, "pattern").ok_or("missing required argument: pattern")?;
            grep::run(
                &pattern,
                arg_str_opt(args, "path").as_deref(),
                arg_str_opt(args, "glob").as_deref(),
                args.get("ignoreCase").and_then(|v| v.as_bool()).unwrap_or(false),
                args.get("literal").and_then(|v| v.as_bool()).unwrap_or(false),
                args.get("context").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize),
                cwd,
            )
        }
        "find" => {
            let pattern = arg_str(args, "pattern").ok_or("missing required argument: pattern")?;
            find::run(&pattern, arg_str_opt(args, "path").as_deref(), args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize), cwd)
        }
        "ls" => {
            ls::run(arg_str_opt(args, "path").as_deref(), args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize), cwd)
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn arg_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn arg_num(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(|v| v.as_f64())
}
