//! CLI argument parsing — port of `coding-agent/src/cli/args.ts` (pi 0.74.2).
//!
//! Same flag surface and semantics; unknown flags are collected like pi does
//! (extension flags), single `-` unknown options are diagnostics errors.

use std::collections::HashMap;

pub const VALID_THINKING_LEVELS: [&str; 6] = ["off", "minimal", "low", "medium", "high", "xhigh"];

#[derive(Debug, Default, Clone)]
pub struct Args {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Vec<String>,
    pub thinking: Option<String>,
    pub continue_: bool,
    pub resume: bool,
    pub help: bool,
    pub version: bool,
    pub mode: Option<String>, // "text" | "json" | "rpc"
    pub no_session: bool,
    pub session: Option<String>,
    pub fork: Option<String>,
    pub session_dir: Option<String>,
    pub models: Vec<String>,
    pub tools: Vec<String>,
    pub no_tools: bool,
    pub no_builtin_tools: bool,
    pub extensions: Vec<String>,
    pub no_extensions: bool,
    pub print: bool,
    pub export: Option<String>,
    pub no_skills: bool,
    pub skills: Vec<String>,
    pub prompt_templates: Vec<String>,
    pub no_prompt_templates: bool,
    pub themes: Vec<String>,
    pub no_themes: bool,
    pub no_context_files: bool,
    pub list_models: Option<Option<String>>, // Some(None) = flag alone, Some(Some(pat)) = with search
    pub offline: bool,
    pub verbose: bool,
    pub messages: Vec<String>,
    pub file_args: Vec<String>,
    pub unknown_flags: HashMap<String, Option<String>>,
    pub diagnostics: Vec<(String, String)>, // (type, message): "warning" | "error"
}

pub fn parse_args<I: IntoIterator<Item = String>>(input: I) -> Args {
    let args: Vec<String> = input.into_iter().collect();
    let mut r = Args::default();

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].clone();

        if arg == "--help" || arg == "-h" {
            r.help = true;
        } else if arg == "--version" || arg == "-v" {
            r.version = true;
        } else if arg == "--mode" && i + 1 < args.len() {
            i += 1;
            let mode = args[i].clone();
            if mode == "text" || mode == "json" || mode == "rpc" {
                r.mode = Some(mode);
            }
        } else if arg == "--continue" || arg == "-c" {
            r.continue_ = true;
        } else if arg == "--resume" || arg == "-r" {
            r.resume = true;
        } else if arg == "--provider" && i + 1 < args.len() {
            i += 1;
            r.provider = Some(args[i].clone());
        } else if arg == "--model" && i + 1 < args.len() {
            i += 1;
            r.model = Some(args[i].clone());
        } else if arg == "--api-key" && i + 1 < args.len() {
            i += 1;
            r.api_key = Some(args[i].clone());
        } else if arg == "--system-prompt" && i + 1 < args.len() {
            i += 1;
            r.system_prompt = Some(args[i].clone());
        } else if arg == "--append-system-prompt" && i + 1 < args.len() {
            i += 1;
            r.append_system_prompt.push(args[i].clone());
        } else if arg == "--no-session" {
            r.no_session = true;
        } else if arg == "--session" && i + 1 < args.len() {
            i += 1;
            r.session = Some(args[i].clone());
        } else if arg == "--fork" && i + 1 < args.len() {
            i += 1;
            r.fork = Some(args[i].clone());
        } else if arg == "--session-dir" && i + 1 < args.len() {
            i += 1;
            r.session_dir = Some(args[i].clone());
        } else if arg == "--models" && i + 1 < args.len() {
            i += 1;
            r.models = args[i].split(',').map(|s| s.trim().to_string()).collect();
        } else if arg == "--no-tools" || arg == "-nt" {
            r.no_tools = true;
        } else if arg == "--no-builtin-tools" || arg == "-nbt" {
            r.no_builtin_tools = true;
        } else if (arg == "--tools" || arg == "-t") && i + 1 < args.len() {
            i += 1;
            r.tools = args[i]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if arg == "--thinking" && i + 1 < args.len() {
            i += 1;
            let level = args[i].clone();
            if VALID_THINKING_LEVELS.contains(&level.as_str()) {
                r.thinking = Some(level);
            } else {
                r.diagnostics.push((
                    "warning".into(),
                    format!(
                        "Invalid thinking level \"{level}\". Valid values: {}",
                        VALID_THINKING_LEVELS.join(", ")
                    ),
                ));
            }
        } else if arg == "--print" || arg == "-p" {
            r.print = true;
            let next = args.get(i + 1);
            if let Some(next) = next {
                if !next.starts_with('@') && (!next.starts_with('-') || next.starts_with("---")) {
                    r.messages.push(next.clone());
                    i += 1;
                }
            }
        } else if arg == "--export" && i + 1 < args.len() {
            i += 1;
            r.export = Some(args[i].clone());
        } else if (arg == "--extension" || arg == "-e") && i + 1 < args.len() {
            i += 1;
            r.extensions.push(args[i].clone());
        } else if arg == "--no-extensions" || arg == "-ne" {
            r.no_extensions = true;
        } else if arg == "--skill" && i + 1 < args.len() {
            i += 1;
            r.skills.push(args[i].clone());
        } else if arg == "--prompt-template" && i + 1 < args.len() {
            i += 1;
            r.prompt_templates.push(args[i].clone());
        } else if arg == "--theme" && i + 1 < args.len() {
            i += 1;
            r.themes.push(args[i].clone());
        } else if arg == "--no-skills" || arg == "-ns" {
            r.no_skills = true;
        } else if arg == "--no-prompt-templates" || arg == "-np" {
            r.no_prompt_templates = true;
        } else if arg == "--no-themes" {
            r.no_themes = true;
        } else if arg == "--no-context-files" || arg == "-nc" {
            r.no_context_files = true;
        } else if arg == "--list-models" {
            let next = args.get(i + 1);
            if let Some(next) = next {
                if !next.starts_with('-') && !next.starts_with('@') {
                    r.list_models = Some(Some(next.clone()));
                    i += 1;
                } else {
                    r.list_models = Some(None);
                }
            } else {
                r.list_models = Some(None);
            }
        } else if arg == "--verbose" {
            r.verbose = true;
        } else if arg == "--offline" {
            r.offline = true;
        } else if let Some(rest) = arg.strip_prefix('@') {
            r.file_args.push(rest.to_string());
        } else if arg.starts_with("--") {
            if let Some(eq) = arg.find('=') {
                r.unknown_flags
                    .insert(arg[2..eq].to_string(), Some(arg[eq + 1..].to_string()));
            } else {
                let flag = arg[2..].to_string();
                let next = args.get(i + 1);
                match next {
                    Some(n) if !n.starts_with('-') && !n.starts_with('@') => {
                        r.unknown_flags.insert(flag, Some(n.clone()));
                        i += 1;
                    }
                    _ => {
                        r.unknown_flags.insert(flag, None);
                    }
                }
            }
        } else if arg.starts_with('-') && !arg.starts_with("--") {
            r.diagnostics.push(("error".into(), format!("Unknown option: {arg}")));
        } else {
            r.messages.push(arg);
        }
        i += 1;
    }

    r
}

pub fn print_help() {
    println!(
        "{} - AI coding assistant with read, bash, edit, write tools

{}:
  pi [options] [@files...] [messages...]

{}:
  --provider <name>              Provider name (default: google)
  --model <pattern>              Model pattern or ID (supports \"provider/id\" and optional \":<thinking>\")
  --api-key <key>                API key (defaults to env vars)
  --system-prompt <text>         System prompt (default: coding assistant prompt)
  --append-system-prompt <text>  Append text or file contents to the system prompt (can be used multiple times)
  --mode <mode>                  Output mode: text (default), json, or rpc
  --print, -p                    Non-interactive mode: process prompt and exit
  --continue, -c                 Continue previous session
  --resume, -r                   Select a session to resume
  --session <path|id>            Use specific session file or partial UUID
  --fork <path|id>               Fork specific session file or partial UUID into a new session
  --session-dir <dir>            Directory for session storage and lookup
  --no-session                   Don't save session (ephemeral)
  --models <patterns>            Comma-separated model patterns for Ctrl+P cycling
  --no-tools, -nt                Disable all tools by default (built-in and extension)
  --no-builtin-tools, -nbt       Disable built-in tools by default but keep extension/custom tools enabled
  --tools, -t <tools>            Comma-separated allowlist of tool names to enable
  --thinking <level>             Set thinking level: off, minimal, low, medium, high, xhigh
  --no-context-files, -nc        Disable AGENTS.md and CLAUDE.md discovery and loading
  --list-models [search]         List available models (with optional fuzzy search)
  --verbose                      Force verbose startup (overrides quietStartup setting)
  --offline                      Disable startup network operations (same as PI_OFFLINE=1)
  --help, -h                     Show this help
  --version, -v                  Show version number

{}:
  DEEPSEEK_API_KEY               - DeepSeek API key
  OPENAI_API_KEY                 - OpenAI GPT API key

{}:
  read   - Read file contents
  bash   - Execute bash commands
  edit   - Edit files with find/replace
  write  - Write files (creates/overwrites)
  grep   - Search file contents (read-only, off by default)
  find   - Find files by glob pattern (read-only, off by default)
  ls     - List directory contents (read-only, off by default)
",
        "pi-rs",
        "Usage:",
        "Options:",
        "Environment Variables:",
        "Built-in Tool Names:"
    );
}
