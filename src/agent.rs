//! Agent loop — port of `packages/agent/src/agent-loop.ts` (pi 0.74.2):
//! request → assistant message → execute tool calls (sequential) → append
//! toolResult messages → repeat until no tool calls / error / abort.

use crate::messages::{ContentBlock, Message, StopReason, ToolResultMessage, UserMessage, AssistantMessage};
use crate::provider;
use crate::session::Session;
use crate::tools;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct AgentConfig {
    pub model: provider::Model,
    pub api_key: String,
    pub system_prompt: String,
    pub tools: Vec<tools::ToolDef>,
    pub cwd: PathBuf,
    pub verbose: bool,
}

pub enum Outcome {
    Completed(AssistantMessage),
    Failed(String),
}

/// Run one prompt to completion. `messages` is the prior conversation
/// (user/assistant/toolResult); the new user message is appended by the caller
/// before invoking this. Returns the final assistant message.
pub fn run_prompt(
    cfg: &AgentConfig,
    messages: &mut Vec<Message>,
    session: &mut Session,
    user_text: &str,
) -> Outcome {
    let user = Message::User(UserMessage {
        role: "user".into(),
        content: vec![crate::messages::text_content(user_text)],
        timestamp: crate::util::now_ms(),
    });
    session.append_message(&user);
    messages.push(user);

    loop {
        let outcome = match provider::stream_chat(&cfg.model, &cfg.api_key, &cfg.system_prompt, messages, &cfg.tools) {
            Ok(o) => o,
            Err(e) => return Outcome::Failed(e),
        };

        let assistant = AssistantMessage {
            role: "assistant".into(),
            content: outcome.content,
            api: cfg.model.api.to_string(),
            provider: cfg.model.provider.clone(),
            model: cfg.model.id.clone(),
            usage: outcome.usage,
            stop_reason: outcome.stop_reason.clone(),
            error_message: outcome.error_message.clone(),
            timestamp: crate::util::now_ms(),
        };
        session.append_message(&Message::Assistant(assistant.clone()));
        let is_err = matches!(assistant.stop_reason, StopReason::Error | StopReason::Aborted);
        messages.push(Message::Assistant(assistant.clone()));
        if is_err {
            return Outcome::Completed(assistant);
        }

        // Extract tool calls in order.
        let tool_calls: Vec<(String, String, Value)> = assistant
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::ToolCall { id, name, arguments } => Some((id.clone(), name.clone(), arguments.clone())),
                _ => None,
            })
            .collect();

        if tool_calls.is_empty() {
            return Outcome::Completed(assistant);
        }

        // Sequential execution (matches pi's `toolExecution: "sequential"` and
        // the built-in tools' file-mutation queue semantics).
        for (id, name, args) in tool_calls {
            if cfg.verbose {
                eprintln!("[tool] {name} {args}");
            }
            let started = std::time::Instant::now();
            // pi: tools outside the enabled set fail immediately with
            // `Tool {name} not found` (agent-loop.ts createErrorToolResult).
            let enabled = cfg.tools.iter().any(|t| t.name == name);
            let result = if enabled {
                tools::execute_tool(&name, &args, &cfg.cwd)
            } else {
                Err(format!("Tool {name} not found"))
            };
            if cfg.verbose {
                eprintln!("[tool] {} done in {:?}", name, started.elapsed());
            }
            let (content, is_error) = match result {
                Ok(text) => (vec![crate::messages::text_content(text)], false),
                Err(e) => (vec![crate::messages::text_content(e)], true),
            };
            let tr = ToolResultMessage {
                role: "toolResult".into(),
                tool_call_id: id,
                tool_name: name,
                content,
                is_error,
                timestamp: crate::util::now_ms(),
            };
            session.append_message(&Message::ToolResult(tr.clone()));
            messages.push(Message::ToolResult(tr));
        }
    }
}

/// Extract the final assistant text (print mode output), mirroring pi's
/// print-mode: concatenate text blocks of the last assistant message; error or
/// aborted goes to stderr with exit code 1.
pub fn final_text(msg: &AssistantMessage) -> Result<String, String> {
    match msg.stop_reason {
        StopReason::Error | StopReason::Aborted => Err(msg
            .error_message
            .clone()
            .unwrap_or_else(|| format!("Request {}", msg.stop_reason.as_str()))),
        _ => {
            let mut out = String::new();
            for c in &msg.content {
                if let ContentBlock::Text { text } = c {
                    out.push_str(text);
                    out.push('\n');
                }
            }
            Ok(out)
        }
    }
}

/// Parse `@file` args into an annotated prompt, like pi's initial-message
/// handling: files are read and prepended as context blocks.
pub fn build_initial_message(messages: &[String], file_args: &[String]) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    for f in file_args {
        let path = Path::new(f);
        if !path.is_file() {
            return Err(format!("File not found: {f}"));
        }
        let content = std::fs::read_to_string(path).map_err(|e| format!("Could not read {f}: {e}"))?;
        parts.push(format!("<file path=\"{f}\">\n{content}\n</file>"));
    }
    parts.extend(messages.iter().cloned());
    Ok(parts.join("\n\n"))
}
