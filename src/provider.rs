//! Provider layer — model catalog + OpenAI-compatible chat-completions streaming.
//!
//! Parity target: pi-ai's `openai-completions` API as used by the `deepseek`
//! provider (`https://api.deepseek.com`), including pi's Usage mapping:
//!   input=prompt_tokens, output=completion_tokens,
//!   cacheRead=prompt_tokens_details.cached_tokens, cacheWrite=0.

use crate::messages::{ContentBlock, Message, StopReason, Usage, UsageCost};
use serde_json::{json, Value};
use std::io::BufRead;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: &'static str, // "openai-completions"
    pub provider: String,
    pub base_url: String,
    pub context_window: u64,
    pub max_tokens: u64,
    pub cost: CostRates,
}

#[derive(Debug, Clone, Copy)]
pub struct CostRates {
    pub input: f64,       // $/M tokens
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Provider registry: known providers, their base URLs and API key env vars.
struct ProviderDef {
    name: &'static str,
    base_url: &'static str,
    env_key: &'static str,
}

const PROVIDERS: &[ProviderDef] = &[
    ProviderDef { name: "deepseek", base_url: "https://api.deepseek.com", env_key: "DEEPSEEK_API_KEY" },
    ProviderDef { name: "openai", base_url: "https://api.openai.com/v1", env_key: "OPENAI_API_KEY" },
];

fn provider_def(name: &str) -> Option<&'static ProviderDef> {
    PROVIDERS.iter().find(|p| p.name == name)
}

pub fn env_key_for_provider(name: &str) -> Option<&'static str> {
    provider_def(name).map(|p| p.env_key)
}

/// Model resolution, mirroring pi's behavior for the flags the CLI accepts:
/// - exact catalog id, or "provider/id", or provider-passthrough for unknown ids.
pub fn resolve_model(provider: Option<&str>, model: Option<&str>) -> Result<Model, String> {
    // "provider/id" form
    if let Some(m) = model {
        if let Some((prov, id)) = m.split_once('/') {
            return passthrough_model(prov, id);
        }
    }
    let prov = provider.unwrap_or("deepseek");
    let id = model.unwrap_or("deepseek-chat");
    // Known catalog ids for deepseek
    let mut m = if prov == "deepseek" {
        match deepseek_catalog(id) {
            Some(m) => m,
            None => passthrough_model(prov, id)?,
        }
    } else {
        passthrough_model(prov, id)?
    };
    // Test/agent override: PI_BASE_URL wins over any catalog default.
    if let Ok(override_url) = std::env::var("PI_BASE_URL") {
        m.base_url = override_url;
    }
    Ok(m)
}

fn passthrough_model(provider: &str, id: &str) -> Result<Model, String> {
    // strip ":thinking" suffix if present
    let id = id.split(':').next().unwrap_or(id);
    let def = provider_def(provider)
        .ok_or_else(|| format!("Unknown provider: {provider}. Known: {}", PROVIDERS.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")))?;
    // Test/agent override: PI_BASE_URL wins over the catalog default.
    let base_url = std::env::var("PI_BASE_URL").unwrap_or_else(|_| def.base_url.to_string());
    Ok(Model {
        id: id.to_string(),
        name: id.to_string(),
        api: "openai-completions",
        provider: provider.to_string(),
        base_url,
        context_window: 128_000,
        max_tokens: 384_000,
        cost: CostRates { input: 0.0, output: 0.0, cache_read: 0.0, cache_write: 0.0 },
    })
}

fn deepseek_catalog(id: &str) -> Option<Model> {
    let (id, rates, ctx, max) = match id {
        "deepseek-chat" => (
            "deepseek-chat",
            CostRates { input: 0.28, output: 0.42, cache_read: 0.028, cache_write: 0.0 },
            131_072,
            384_000,
        ),
        "deepseek-reasoner" => (
            "deepseek-reasoner",
            CostRates { input: 0.55, output: 2.19, cache_read: 0.14, cache_write: 0.0 },
            128_000,
            64_000,
        ),
        "deepseek-v4-flash" => (
            "deepseek-v4-flash",
            CostRates { input: 0.14, output: 0.28, cache_read: 0.0028, cache_write: 0.0 },
            1_000_000,
            384_000,
        ),
        "deepseek-v4-pro" => (
            "deepseek-v4-pro",
            CostRates { input: 0.435, output: 0.87, cache_read: 0.003625, cache_write: 0.0 },
            1_000_000,
            384_000,
        ),
        _ => return None,
    };
    Some(Model {
        id: id.to_string(),
        name: id.to_string(),
        api: "openai-completions",
        provider: "deepseek".into(),
        base_url: "https://api.deepseek.com".into(),
        context_window: ctx,
        max_tokens: max,
        cost: rates,
    })
}

pub fn list_models() -> Vec<Model> {
    ["deepseek-chat", "deepseek-reasoner", "deepseek-v4-flash", "deepseek-v4-pro"]
        .iter()
        .filter_map(|id| deepseek_catalog(id))
        .collect()
}

// ---------------------------------------------------------------------------
// Wire types for the chat-completions stream
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StreamOutcome {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    pub error_message: Option<String>,
}

fn usage_cost(m: &Model, u: &Usage) -> UsageCost {
    let mtk = 1_000_000.0;
    let c = &m.cost;
    UsageCost {
        input: u.input as f64 / mtk * c.input,
        output: u.output as f64 / mtk * c.output,
        cache_read: u.cache_read as f64 / mtk * c.cache_read,
        cache_write: u.cache_write as f64 / mtk * c.cache_write,
        total: 0.0,
    }
    .with_total()
}

impl UsageCost {
    fn with_total(mut self) -> Self {
        self.total = self.input + self.output + self.cache_read + self.cache_write;
        self
    }
}

/// Convert session messages to OpenAI chat-completions wire messages.
/// - system prompt is passed separately (`system` param)
/// - assistant toolCall blocks -> `tool_calls`
/// - toolResult -> role "tool" with tool_call_id
fn to_wire_messages(messages: &[Message], system: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if !system.is_empty() {
        // pi-ai uses role "developer" when compat.supportsDeveloperRole is set
        // in the model catalog. The live catalog currently marks deepseek as
        // supporting it, but the real DeepSeek API rejects it (HTTP 422:
        // unknown variant), so pi-rs always sends "system" here.
        out.push(json!({ "role": "system", "content": system }));
    }
    for m in messages {
        match m {
            Message::User(u) => {
                // User content is an array of content blocks.
                let blocks: Vec<Value> = u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
                        _ => None,
                    })
                    .collect();
                out.push(json!({ "role": "user", "content": blocks }));
            }
            Message::Assistant(a) => {
                let mut text_parts: Vec<String> = Vec::new();
                let mut thinking_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                for c in &a.content {
                    match c {
                        ContentBlock::Text { text } => text_parts.push(text.clone()),
                        ContentBlock::Thinking { thinking } => thinking_parts.push(thinking.clone()),
                        ContentBlock::ToolCall { id, name, arguments } => {
                            tool_calls.push(json!({
                                "id": id, "type": "function",
                                "function": { "name": name, "arguments": serde_json::to_string(arguments).unwrap_or_default() }
                            }));
                        }
                        ContentBlock::Image { .. } => {}
                    }
                }
                let content = if text_parts.is_empty() {
                    Value::Null
                } else {
                    json!(text_parts.join(""))
                };
                let mut obj = json!({ "role": "assistant", "content": content, "reasoning_content": thinking_parts.join("") });
                if !tool_calls.is_empty() {
                    obj["tool_calls"] = Value::Array(tool_calls);
                }
                out.push(obj);
            }
            Message::ToolResult(t) => {
                let text: Vec<String> = t
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect();
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": t.tool_call_id,
                    "content": text.join("\n"),
                }));
            }
        }
    }
    out
}

fn tool_schema_json(tools: &[crate::tools::ToolDef]) -> Vec<Value> {
    // 1.4.0 wire (captured): the four coding tools carry strict:true with
    // strict-mode schemas; grep/find/ls keep strict:false and plain schemas.
    tools
        .iter()
        .map(|t| {
            let strict = matches!(t.name, "read" | "bash" | "edit" | "write");
            json!({
                "type": "function",
                "function": { "name": t.name, "description": t.description, "parameters": t.parameters, "strict": strict }
            })
        })
        .collect()
}

/// One streaming chat-completions request. Blocking; SSE lines parsed incrementally.
/// Tool calls are accumulated across deltas by index, mirroring pi-ai behavior.
pub fn stream_chat(
    model: &Model,
    api_key: &str,
    system: &str,
    messages: &[Message],
    tools: &[crate::tools::ToolDef],
) -> Result<StreamOutcome, String> {
    stream_chat_cb(model, api_key, system, messages, tools, &mut |_| {})
}

/// Same as `stream_chat`, forwarding assistant text deltas as they arrive
/// (interactive mode). `on_text` receives incremental content chunks.
pub fn stream_chat_cb(
    model: &Model,
    api_key: &str,
    system: &str,
    messages: &[Message],
    tools: &[crate::tools::ToolDef],
    on_text: &mut dyn FnMut(&str),
) -> Result<StreamOutcome, String> {
    let url = format!("{}/chat/completions", model.base_url.trim_end_matches('/'));
    let thinking_level = std::env::var("PI_THINKING_LEVEL").unwrap_or_else(|_| "high".into());
    let mut body = json!({
        "model": model.id,
        "messages": to_wire_messages(messages, system),
        "stream": true,
        "stream_options": { "include_usage": true },
        "max_tokens": model.max_tokens,
        "tools": tool_schema_json(tools),
    });
    // pi-ai deepseek compat: reasoning_effort + thinking decoration (chat
    // endpoint ignores them; sent for wire parity with pi 0.74.2).
    if model.provider == "deepseek" && model.api == "openai-completions" {
        if thinking_level != "off" {
            body["reasoning_effort"] = json!(thinking_level);
            body["thinking"] = json!({ "type": "enabled" });
        }
    }

    let agent: ureq::Agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .build();

    let resp = agent
        .post(&url)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream")
        .send_string(&body.to_string())
        .map_err(|e| match e {
            ureq::Error::Status(code, r) => {
                let body = r.into_string().unwrap_or_default();
                format!("Provider request failed (HTTP {code}): {}", truncate_err(&body))
            }
            other => format!("Provider request failed: {other}"),
        })?;

    let reader = std::io::BufReader::new(resp.into_reader());

    let mut text_acc = String::new();
    let mut thinking_acc = String::new();
    // tool call accumulation: (index) -> (id, name, arguments)
    let mut tc_order: Vec<usize> = Vec::new();
    let mut tc: std::collections::HashMap<usize, (String, String, String)> =
        std::collections::HashMap::new();
    let mut finish_reason: Option<String> = None;
    let mut usage_raw: Option<Value> = None;
    let mut error_message: Option<String> = None;

    for line in reader.lines() {
        let line = line.map_err(|e| format!("Stream read error: {e}"))?;
        let line = line.trim_start();
        if !(line.starts_with("data:") || line.starts_with("data: ")) {
            continue;
        }
        let payload = line[5..].trim();
        if payload == "[DONE]" {
            break;
        }
        let v: Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(err) = v.get("error") {
            error_message = Some(
                err.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown provider error")
                    .to_string(),
            );
            break;
        }
        if let Some(u) = v.get("usage") {
            if u.is_object() {
                usage_raw = Some(u.clone());
            }
        }
        let choices = match v.get("choices").and_then(|c| c.as_array()) {
            Some(c) => c,
            None => continue,
        };
        for choice in choices {
            let delta = choice.get("delta");
            if let Some(d) = delta {
                if let Some(rc) = d.get("reasoning_content").and_then(|x| x.as_str()) {
                    thinking_acc.push_str(rc);
                }
                if let Some(c) = d.get("content") {
                    if let Some(s) = c.as_str() {
                        text_acc.push_str(s);
                        on_text(s);
                    }
                }
                if let Some(tcs) = d.get("tool_calls").and_then(|x| x.as_array()) {
                    for t in tcs {
                        let idx = t.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                        let entry = tc.entry(idx).or_insert_with(|| {
                            tc_order.push(idx);
                            (String::new(), String::new(), String::new())
                        });
                        if let Some(id) = t.get("id").and_then(|x| x.as_str()) {
                            if !id.is_empty() {
                                entry.0 = id.to_string();
                            }
                        }
                        if let Some(f) = t.get("function") {
                            if let Some(n) = f.get("name").and_then(|x| x.as_str()) {
                                if !n.is_empty() {
                                    entry.1 = n.to_string();
                                }
                            }
                            if let Some(a) = f.get("arguments").and_then(|x| x.as_str()) {
                                entry.2.push_str(a);
                            }
                        }
                    }
                }
            }
            if let Some(fr) = choice.get("finish_reason").and_then(|x| x.as_str()) {
                if !fr.is_empty() {
                    finish_reason = Some(fr.to_string());
                }
            }
        }
    }

    if let Some(em) = error_message {
        return Ok(StreamOutcome {
            content: vec![],
            stop_reason: StopReason::Error,
            usage: Usage::default(),
            error_message: Some(em),
        });
    }

    let mut content: Vec<ContentBlock> = Vec::new();
    if !thinking_acc.is_empty() {
        content.push(ContentBlock::Thinking { thinking: std::mem::take(&mut thinking_acc) });
    }
    if !text_acc.is_empty() {
        content.push(ContentBlock::Text { text: std::mem::take(&mut text_acc) });
    }
    tc_order.sort_unstable();
    let mut has_tool_call = false;
    for idx in &tc_order {
        if let Some((id, name, args)) = tc.get(idx) {
            has_tool_call = true;
            let arguments: Value = if args.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(args).unwrap_or(json!({ "_raw": args }))
            };
            content.push(ContentBlock::ToolCall {
                id: if id.is_empty() { crate::util::short_id() } else { id.clone() },
                name: name.clone(),
                arguments,
            });
        }
    }

    let stop_reason = if has_tool_call {
        StopReason::ToolUse
    } else {
        match finish_reason.as_deref() {
            Some("length") => StopReason::Length,
            _ => StopReason::Stop,
        }
    };

    let usage = usage_raw
        .map(|u| {
            let prompt_total = u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
            let output = u.get("completion_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
            let cache_read = u
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            // pi-ai semantics: Usage.input EXCLUDES cached tokens.
            let input = prompt_total.saturating_sub(cache_read);
            let reasoning = u
                .pointer("/completion_tokens_details/reasoning_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let total_out = output + reasoning;
            let cost = usage_cost(
                model,
                &Usage { input, output: total_out, cache_read, cache_write: 0, reasoning: 0, total_tokens: 0, cost: None },
            );
            // pi: totalTokens = raw prompt_tokens + completion_tokens (cache not deducted)
            crate::messages::usage_from_parts(input, total_out, cache_read, 0, Some(cost))
                .with_total(prompt_total + total_out)
        })
        .unwrap_or_default();

    Ok(StreamOutcome { content, stop_reason, usage, error_message: None })
}

fn truncate_err(s: &str) -> String {
    if s.len() > 400 {
        format!("{}...", &s[..400])
    } else {
        s.to_string()
    }
}
