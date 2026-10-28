//! Message model — mirrors `@earendil-works/pi-ai` types (pi 0.74.2) so session
//! JSONL stays field-compatible (roles, content blocks, usage, stopReason).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    #[serde(rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: f64,
    pub total: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    #[serde(rename = "cacheRead")]
    pub cache_read: u64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: u64,
    /// pi v1.4.0 emits this on session usage (0 unless the provider reports it).
    #[serde(default)]
    pub reasoning: u64,
    #[serde(rename = "totalTokens")]
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<UsageCost>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "toolCall")]
    ToolCall { id: String, name: String, arguments: Value },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

impl StopReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            StopReason::Stop => "stop",
            StopReason::Length => "length",
            StopReason::ToolUse => "toolUse",
            StopReason::Error => "error",
            StopReason::Aborted => "aborted",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub role: String, // "assistant"
    pub content: Vec<ContentBlock>,
    pub api: String,      // e.g. "openai-completions"
    pub provider: String, // e.g. "deepseek"
    pub model: String,
    pub usage: Usage,
    #[serde(rename = "stopReason")]
    pub stop_reason: StopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessage {
    pub role: String, // "toolResult"
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub content: Vec<ContentBlock>,
    #[serde(rename = "isError")]
    pub is_error: bool,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMessage {
    pub role: String, // "user"
    pub content: Vec<ContentBlock>,
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub enum Message {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

impl Message {
    pub fn timestamp(&self) -> u64 {
        match self {
            Message::User(m) => m.timestamp,
            Message::Assistant(m) => m.timestamp,
            Message::ToolResult(m) => m.timestamp,
        }
    }
}

// Transparent (de)serialization: the JSON shape is exactly the inner message
// struct (pi session entries store the bare message object with its role).
impl Serialize for Message {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Message::User(m) => m.serialize(s),
            Message::Assistant(m) => m.serialize(s),
            Message::ToolResult(m) => m.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        match v.get("role").and_then(|r| r.as_str()) {
            Some("user") => serde_json::from_value::<UserMessage>(v)
                .map(Message::User)
                .map_err(serde::de::Error::custom),
            Some("assistant") => serde_json::from_value::<AssistantMessage>(v)
                .map(Message::Assistant)
                .map_err(serde::de::Error::custom),
            Some("toolResult") => serde_json::from_value::<ToolResultMessage>(v)
                .map(Message::ToolResult)
                .map_err(serde::de::Error::custom),
            other => Err(serde::de::Error::custom(format!(
                "unknown message role: {:?}",
                other.unwrap_or("missing")
            ))),
        }
    }
}

pub fn text_content(s: impl Into<String>) -> ContentBlock {
    ContentBlock::Text { text: s.into() }
}

pub fn usage_from_parts(
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cost: Option<UsageCost>,
) -> Usage {
    Usage {
        input,
        output,
        cache_read,
        cache_write,
        reasoning: 0,
        total_tokens: input + output,
        cost,
    }
}

impl Usage {
    /// Override totalTokens (pi uses raw prompt+completion, cache not deducted).
    pub fn with_total(mut self, total: u64) -> Usage {
        self.total_tokens = total;
        self
    }
}
