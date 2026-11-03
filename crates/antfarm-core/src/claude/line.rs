//! Serde types for the JSONL transcript lines antfarm reads.
//!
//! Every struct tolerates missing and unknown fields. Only the fields antfarm uses are
//! declared; everything else is skipped by serde without being kept.

use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::Deserialize;

use crate::model::Time;

/// Fields shared by `assistant`, `user`, and `system` lines.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Envelope {
    pub uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub timestamp: Option<Time>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub version: Option<String>,
    pub entrypoint: Option<String>,
    pub is_sidechain: bool,
    pub agent_id: Option<String>,
    pub requested_model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AssistantLine {
    /// Filled by a separate pass; `flatten` would buffer the whole line.
    #[serde(skip)]
    pub env: Envelope,
    pub message: AssistantMessage,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AssistantMessage {
    pub id: Option<String>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<RawUsage>,
    pub content: Vec<Block>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct RawUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub output_tokens_details: Option<OutputDetails>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct OutputDetails {
    pub thinking_tokens: u64,
}

/// One content block. Unknown block types become `Other`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        #[serde(default)]
        text: String,
    },
    Thinking {},
    RedactedThinking {},
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        is_error: Option<bool>,
        #[serde(default, deserialize_with = "byte_len")]
        content: u64,
    },
    Image {},
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UserLine {
    /// Filled by a separate pass; `flatten` would buffer the whole line.
    #[serde(skip)]
    pub env: Envelope,
    pub message: UserMessage,
    pub tool_use_result: Option<ToolUseResult>,
    pub is_meta: bool,
    pub is_compact_summary: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserMessage {
    pub content: UserContent,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Blocks(Vec<Block>),
}

impl Default for UserContent {
    fn default() -> Self {
        Self::Blocks(Vec::new())
    }
}

/// The handful of `toolUseResult` fields antfarm uses. The field is sometimes a list or a
/// string instead of an object; those decode to the default.
#[derive(Debug, Clone, Default)]
pub struct ToolUseResult {
    pub is_async: bool,
    pub status: Option<String>,
    pub agent_id: Option<String>,
    pub file_path: Option<String>,
}

impl<'de> Deserialize<'de> for ToolUseResult {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ToolUseResult;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ToolUseResult, A::Error> {
                let mut out = ToolUseResult::default();
                while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
                    match key.as_ref() {
                        "isAsync" => out.is_async = map.next_value::<Option<bool>>()?.unwrap_or(false),
                        "status" => out.status = map.next_value::<Option<OptString>>()?.and_then(|s| s.0),
                        "agentId" => out.agent_id = map.next_value::<Option<OptString>>()?.and_then(|s| s.0),
                        "filePath" => out.file_path = map.next_value::<Option<OptString>>()?.and_then(|s| s.0),
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<ToolUseResult, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(ToolUseResult::default())
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<ToolUseResult, E> {
                Ok(ToolUseResult::default())
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<ToolUseResult, E> {
                Ok(ToolUseResult::default())
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<ToolUseResult, E> {
                Ok(ToolUseResult::default())
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<ToolUseResult, E> {
                Ok(ToolUseResult::default())
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<ToolUseResult, E> {
                Ok(ToolUseResult::default())
            }
            fn visit_unit<E: de::Error>(self) -> Result<ToolUseResult, E> {
                Ok(ToolUseResult::default())
            }
        }
        d.deserialize_any(V)
    }
}

/// A string field that may hold a non-string; non-strings decode to `None`.
struct OptString(Option<String>);

impl<'de> Deserialize<'de> for OptString {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(Self(match v {
            serde_json::Value::String(s) => Some(s),
            _ => None,
        }))
    }
}

/// Counts the bytes of text in a tool result without keeping it.
fn byte_len<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    fn len(v: &serde_json::Value) -> u64 {
        match v {
            serde_json::Value::String(s) => s.len() as u64,
            serde_json::Value::Array(a) => a.iter().map(len).sum(),
            serde_json::Value::Object(o) => o.iter().filter(|(k, _)| k.as_str() != "type").map(|(_, v)| len(v)).sum(),
            _ => 0,
        }
    }
    let v = serde_json::Value::deserialize(d)?;
    Ok(len(&v))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SystemLine {
    /// Filled by a separate pass; `flatten` would buffer the whole line.
    #[serde(skip)]
    pub env: Envelope,
    pub subtype: Option<String>,
    pub compact_metadata: Option<CompactMetadata>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompactMetadata {
    pub trigger: Option<String>,
    pub pre_tokens: Option<u64>,
    pub post_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CostStateLine {
    #[serde(rename = "totalCostUSD")]
    pub total_cost_usd: Option<f64>,
    pub total_lines_added: Option<u64>,
    pub total_lines_removed: Option<u64>,
    pub model_usage: Option<std::collections::BTreeMap<String, ModelUsage>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    #[serde(rename = "costUSD")]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TitleLine {
    pub agent_name: Option<String>,
    pub custom_title: Option<String>,
    pub ai_title: Option<String>,
    pub last_prompt: Option<String>,
}
