//! pi-rs: Rust port of the pi coding agent (parity with 0.74.2 headless core).
//!
//! Modes:
//! - `-p "prompt"`            print mode: run prompt, print final text, exit
//! - `--mode json "prompt"`   print mode emitting the session event stream
//! - non-TTY with a prompt    one-shot print mode (pi's auto-headless behavior)
//! - interactive TTY          not implemented yet (TUI is on the roadmap);
//!                            prints an error and exits 1.

mod agent;
mod args;
mod context;
mod messages;
mod provider;
mod session;
mod system_prompt;
mod tools;
mod util;

use args::Args;
use messages::Message;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let a = args::parse_args(argv);

    if a.help {
        args::print_help();
        return ExitCode::SUCCESS;
    }
    if a.version {
        println!("0.74.2");
        return ExitCode::SUCCESS;
    }
    for (t, m) in &a.diagnostics {
        eprintln!("{t}: {m}");
        if t == "error" {
            return ExitCode::from(1);
        }
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cwd_str = cwd.to_string_lossy().into_owned();

    // Model + API key resolution (pi: --api-key > env var per provider).
    let model = match provider::resolve_model(a.provider.as_deref(), a.model.as_deref()) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let api_key = match &a.api_key {
        Some(k) => k.clone(),
        None => {
            let env_var = provider::env_key_for_provider(&model.provider).unwrap_or("OPENAI_API_KEY");
            match std::env::var(env_var) {
                Ok(k) if !k.is_empty() => k,
                _ => {
                    eprintln!(
                        "No API key for provider {}. Set {} or use --api-key.",
                        model.provider, env_var
                    );
                    return ExitCode::from(1);
                }
            }
        }
    };

    // Prompt assembly: messages joined; @file args inlined.
    if a.messages.is_empty() && a.file_args.is_empty() && !a.continue_ {
        eprintln!("No prompt given. Usage: pi -p \"prompt\"  (interactive TUI not built in pi-rs yet)");
        return ExitCode::from(1);
    }
    let user_text = match agent::build_initial_message(&a.messages, &a.file_args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };

    // Tools: default coding tools; --tools allowlist; --no-tools disables all.
    let selected: Vec<String> = if a.no_tools || a.no_builtin_tools {
        Vec::new()
    } else if !a.tools.is_empty() {
        a.tools.clone()
    } else {
        tools::DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect()
    };
    let tool_defs: Vec<tools::ToolDef> = selected.iter().filter_map(|t| tools::tool_definition(t)).collect();
    let snippets: Vec<(&str, &str)> = selected
        .iter()
        .filter_map(|t| tools::snippet_for(t).map(|s| (t.as_str(), s)))
        .collect();

    // Context files (AGENTS.md / CLAUDE.md).
    let context_files = if a.no_context_files {
        Vec::new()
    } else {
        let agent_dir = session::home_dir().join(".pi").join("agent");
        context::load_project_context_files(&cwd, &agent_dir)
    };

    let system = system_prompt::build_system_prompt(system_prompt::SystemPromptOptions {
        custom_prompt: a.system_prompt.as_deref(),
        append_system_prompt: if a.append_system_prompt.is_empty() {
            None
        } else {
            Some(a.append_system_prompt.join("\n\n").leak() as &str)
        },
        selected_tools: &selected,
        tool_snippets: &snippets,
        cwd: &cwd_str,
        context_files: &context_files,
    });

    let cfg = agent::AgentConfig {
        model: model.clone(),
        api_key,
        system_prompt: system,
        tools: tool_defs,
        cwd: cwd.clone(),
        verbose: a.verbose,
    };

    // Session setup.
    let persist = !a.no_session;
    let session_dir: Option<&Path> = a.session_dir.as_deref().map(Path::new);
    let mut session = match session::Session::create(session_dir, &cwd, persist) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not create session: {e}");
            return ExitCode::from(1);
        }
    };

    session.append_setup_entries(&model.provider, &model.id, "high");

    let mut history: Vec<Message> = Vec::new();
    if a.continue_ {
        let dir = session_dir
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| session_dir_default(&cwd));
        if let Some(f) = session::find_most_recent_session(&dir) {
            match session::load_messages(&f) {
                Ok(msgs) => history = msgs,
                Err(e) => eprintln!("Warning: could not load previous session: {e}"),
            }
        }
    }

    let json_mode = a.mode.as_deref() == Some("json");
    if json_mode {
        // pi emits the session header first in json mode.
        println!(
            "{}",
            serde_json::json!({
                "type": "session",
                "version": session::CURRENT_SESSION_VERSION,
                "id": session.id,
                "cwd": cwd_str,
            })
        );
    }

    let started = std::time::Instant::now();
    let outcome = agent::run_prompt(&cfg, &mut history, &mut session, &user_text);

    match outcome {
        agent::Outcome::Completed(msg) => {
            if a.verbose {
                eprintln!(
                    "[stats] turns done in {:?}; tokens in={} out={} cache_read={}",
                    started.elapsed(),
                    msg.usage.input,
                    msg.usage.output,
                    msg.usage.cache_read
                );
            }
            if json_mode {
                println!("{}", serde_json::to_string(&msg).unwrap_or_default());
                return ExitCode::SUCCESS;
            }
            match agent::final_text(&msg) {
                Ok(text) => {
                    print!("{text}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        agent::Outcome::Failed(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn session_dir_default(cwd: &Path) -> PathBuf {
    let cwd_str = cwd.to_string_lossy().replace('\\', "/");
    let safe = format!("--{}--", cwd_str.trim_start_matches('/').replace(['/', ':'], "-"));
    session::home_dir().join(".pi").join("agent").join("sessions").join(safe)
}

#[allow(dead_code)]
fn _assert_tty_detection() {
    let _ = std::io::stdin().is_terminal();
}
