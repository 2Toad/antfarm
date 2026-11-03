//! Serde types for the JSONL transcript lines antfarm reads.
//!
//! Every struct tolerates missing and unknown fields. Only the fields antfarm uses are
//! declared; everything else is skipped by serde without being kept.

use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};

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
    /// How the cache writes split between the 5 minute and 1 hour caches, when the API
    /// reported it. Older lines have only the total.
    pub cache_creation: Option<CacheCreation>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct CacheCreation {
    pub ephemeral_5m_input_tokens: u64,
    pub ephemeral_1h_input_tokens: u64,
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
        #[serde(default, deserialize_with = "result_body")]
        content: ResultBody,
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
    /// Set when a Bash call was sent to the background; it later finishes with a
    /// task notification carrying this id.
    pub background_task_id: Option<String>,
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
                        "isAsync" => {
                            out.is_async = map.next_value::<Option<bool>>()?.unwrap_or(false)
                        }
                        "status" => {
                            out.status = map.next_value::<Option<OptString>>()?.and_then(|s| s.0)
                        }
                        "agentId" => {
                            out.agent_id = map.next_value::<Option<OptString>>()?.and_then(|s| s.0)
                        }
                        "backgroundTaskId" => {
                            out.background_task_id =
                                map.next_value::<Option<OptString>>()?.and_then(|s| s.0)
                        }
                        "filePath" => {
                            out.file_path = map.next_value::<Option<OptString>>()?.and_then(|s| s.0)
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<ToolUseResult, A::Error> {
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
/// What antfarm keeps of a tool result: its size, and for a background command the file
/// its output is written to. The text itself is not kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultBody {
    pub bytes: u64,
    pub output_file: Option<String>,
}

/// The marker Claude Code writes in a background command's result.
const OUTPUT_MARKER: &str = "Output is being written to: ";

fn result_body<'de, D: Deserializer<'de>>(d: D) -> Result<ResultBody, D::Error> {
    fn walk(v: &serde_json::Value, out: &mut ResultBody) {
        match v {
            serde_json::Value::String(s) => {
                out.bytes += s.len() as u64;
                if out.output_file.is_none()
                    && let Some(i) = s.find(OUTPUT_MARKER)
                {
                    let rest = &s[i + OUTPUT_MARKER.len()..];
                    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                    let path = rest[..end].trim_end_matches('.');
                    if path.starts_with('/') {
                        out.output_file = Some(path.to_owned());
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            serde_json::Value::Object(o) => o
                .iter()
                .filter(|(k, _)| k.as_str() != "type")
                .for_each(|(_, x)| walk(x, out)),
            _ => {}
        }
    }
    let v = serde_json::Value::deserialize(d)?;
    let mut out = ResultBody::default();
    walk(&v, &mut out);
    Ok(out)
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
    /// Absent on every line seen so far; kept in case a build adds it.
    pub timestamp: Option<Time>,
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
    pub cache_creation_ephemeral_5m_input_tokens: Option<u64>,
    pub cache_creation_ephemeral_1h_input_tokens: Option<u64>,
    pub thinking_tokens: Option<u64>,
    pub web_search_requests: Option<u64>,
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
