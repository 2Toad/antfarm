//! Turns one raw JSONL line into a [`Parsed`] value, failing soft.

use std::borrow::Cow;

use serde::Deserialize;

use super::line::{AssistantLine, CostStateLine, Envelope, SystemLine, TitleLine, UserLine};
use crate::model::ParseStats;

/// Line types antfarm decodes. Everything else is counted and skipped.
pub const WANTED: &[&str] = &[
    "assistant",
    "user",
    "system",
    "cost-state",
    "agent-name",
    "custom-title",
    "ai-title",
    "last-prompt",
];

#[derive(Debug, Clone)]
pub enum Parsed {
    Assistant(Box<AssistantLine>),
    User(Box<UserLine>),
    System(Box<SystemLine>),
    CostState(Box<CostStateLine>),
    AgentName(String),
    CustomTitle(String),
    AiTitle(String),
    LastPrompt(String),
    /// A line of a type antfarm does not use.
    Ignored(String),
}

impl Parsed {
    pub fn version(&self) -> Option<&str> {
        match self {
            Parsed::Assistant(l) => l.env.version.as_deref(),
            Parsed::User(l) => l.env.version.as_deref(),
            Parsed::System(l) => l.env.version.as_deref(),
            _ => None,
        }
    }
}

/// A complete line read from a transcript, with where it came from.
#[derive(Debug, Clone)]
pub struct RawLine {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub offset: u64,
    pub len: u64,
    pub parsed: Parsed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub offset: u64,
    pub message: String,
}

#[derive(Deserialize)]
struct TypeOnly<'a> {
    #[serde(borrow, rename = "type")]
    kind: Option<Cow<'a, str>>,
}

/// Reads the `type` of a line cheaply when it is the first key, as it is for every
/// metadata line Claude Code writes.
fn type_prefix(bytes: &[u8]) -> Option<&str> {
    let rest = bytes.strip_prefix(br#"{"type":""#)?;
    let end = memchr::memchr(b'"', rest)?;
    std::str::from_utf8(&rest[..end]).ok()
}

pub fn parse_line(bytes: &[u8], offset: u64) -> Result<Parsed, ParseError> {
    let err = |e: serde_json::Error| ParseError {
        offset,
        message: e.to_string(),
    };
    let bytes = trim_cr(bytes);

    let kind: String = match type_prefix(bytes) {
        Some(t) if !WANTED.contains(&t) => return Ok(Parsed::Ignored(t.to_owned())),
        Some(t) => t.to_owned(),
        None => {
            let peek: TypeOnly = serde_json::from_slice(bytes).map_err(err)?;
            match peek.kind {
                Some(t) => t.into_owned(),
                None => {
                    return Err(ParseError {
                        offset,
                        message: "line has no type".into(),
                    });
                }
            }
        }
    };

    let env = || serde_json::from_slice::<Envelope>(bytes).map_err(err);
    Ok(match kind.as_str() {
        "assistant" => {
            let mut l: AssistantLine = serde_json::from_slice(bytes).map_err(err)?;
            l.env = env()?;
            Parsed::Assistant(Box::new(l))
        }
        "user" => {
            let mut l: UserLine = serde_json::from_slice(bytes).map_err(err)?;
            l.env = env()?;
            Parsed::User(Box::new(l))
        }
        "system" => {
            let mut l: SystemLine = serde_json::from_slice(bytes).map_err(err)?;
            l.env = env()?;
            Parsed::System(Box::new(l))
        }
        "cost-state" => Parsed::CostState(Box::new(serde_json::from_slice(bytes).map_err(err)?)),
        "agent-name" | "custom-title" | "ai-title" | "last-prompt" => {
            let t: TitleLine = serde_json::from_slice(bytes).map_err(err)?;
            match kind.as_str() {
                "agent-name" => Parsed::AgentName(t.agent_name.unwrap_or_default()),
                "custom-title" => Parsed::CustomTitle(t.custom_title.unwrap_or_default()),
                "ai-title" => Parsed::AiTitle(t.ai_title.unwrap_or_default()),
                _ => Parsed::LastPrompt(t.last_prompt.unwrap_or_default()),
            }
        }
        _ => Parsed::Ignored(kind),
    })
}

fn trim_cr(b: &[u8]) -> &[u8] {
    b.strip_suffix(b"\r").unwrap_or(b)
}

/// Parses a batch of raw lines, updating counters, and returns the good ones and the errors.
pub fn parse_batch(
    lines: Vec<RawLine>,
    stats: &mut ParseStats,
) -> (Vec<ParsedLine>, Vec<ParseError>) {
    let mut good = Vec::with_capacity(lines.len());
    let mut bad = Vec::new();
    for raw in lines {
        let len = raw.bytes.len() as u64 + 1;
        stats.lines += 1;
        stats.bytes += len;
        if raw.bytes.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        match parse_line(&raw.bytes, raw.offset) {
            Ok(parsed) => {
                match &parsed {
                    Parsed::Ignored(t) => *stats.ignored_types.entry(t.clone()).or_default() += 1,
                    p => {
                        stats.parsed += 1;
                        if let Some(v) = p.version() {
                            *stats.versions.entry(v.to_owned()).or_default() += 1;
                        }
                    }
                }
                good.push(ParsedLine {
                    offset: raw.offset,
                    len,
                    parsed,
                });
            }
            Err(e) => {
                stats.errors += 1;
                if stats.first_error.is_none() {
                    stats.first_error = Some(format!("offset {}: {}", e.offset, e.message));
                }
                bad.push(e);
            }
        }
    }
    (good, bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::line::{Block, UserContent};

    fn p(s: &str) -> Parsed {
        parse_line(s.as_bytes(), 0).expect("parses")
    }

    #[test]
    fn assistant_with_tool_use_and_usage() {
        let line = r#"{"parentUuid":"a","isSidechain":false,"type":"assistant","uuid":"u1","timestamp":"2026-11-02T00:21:58.124Z","sessionId":"s","version":"2.1.314","requestedModel":"opus","effort":"high","newField":{"x":1},"message":{"id":"msg_1","model":"claude-opus-5-5","role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls","description":"list"}}],"usage":{"input_tokens":2,"cache_creation_input_tokens":33,"cache_read_input_tokens":24,"output_tokens":245,"output_tokens_details":{"thinking_tokens":58}}}}"#;
        let Parsed::Assistant(a) = p(line) else {
            panic!("not assistant")
        };
        assert_eq!(a.message.id.as_deref(), Some("msg_1"));
        assert_eq!(a.env.version.as_deref(), Some("2.1.314"));
        assert_eq!(a.env.requested_model.as_deref(), Some("opus"));
        let u = a.message.usage.unwrap();
        assert_eq!(
            (u.input_tokens, u.cache_read_input_tokens, u.output_tokens),
            (2, 24, 245)
        );
        assert_eq!(u.output_tokens_details.unwrap().thinking_tokens, 58);
        match &a.message.content[0] {
            Block::ToolUse { id, name, input } => {
                assert_eq!((id.as_str(), name.as_str()), ("toolu_1", "Bash"));
                assert_eq!(input["command"], "ls");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn assistant_thinking_text_image_and_unknown_blocks() {
        let line = r#"{"type":"assistant","message":{"id":"m","content":[{"type":"thinking","thinking":"hmm","signature":"abc"},{"type":"text","text":"hi"},{"type":"image","source":{"type":"base64","data":"AAAA"}},{"type":"brand_new_block","x":1}]}}"#;
        let Parsed::Assistant(a) = p(line) else {
            panic!()
        };
        let kinds: Vec<_> = a.message.content.iter().map(|b| format!("{b:?}")).collect();
        assert!(
            matches!(a.message.content[0], Block::Thinking {}),
            "{kinds:?}"
        );
        assert!(matches!(&a.message.content[1], Block::Text { text } if text == "hi"));
        assert!(matches!(a.message.content[2], Block::Image {}));
        assert!(matches!(a.message.content[3], Block::Other));
    }

    #[test]
    fn user_tool_result_and_async_agent_result() {
        let line = r#"{"type":"user","uuid":"u2","message":{"role":"user","content":[{"tool_use_id":"toolu_1","type":"tool_result","content":"hello world","is_error":false}]},"toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"a47","description":"d","resolvedModel":"sonnet"},"permissionDecision":{"decision":"allow","source":"rule"}}"#;
        let Parsed::User(u) = p(line) else { panic!() };
        let UserContent::Blocks(b) = &u.message.content else {
            panic!()
        };
        match &b[0] {
            Block::ToolResult {
                tool_use_id,
                is_error,
                content,
            } => {
                assert_eq!(tool_use_id, "toolu_1");
                assert_eq!(*is_error, Some(false));
                assert_eq!(*content, 11);
            }
            other => panic!("{other:?}"),
        }
        let r = u.tool_use_result.as_ref().unwrap();
        assert!(r.is_async);
        assert_eq!(r.agent_id.as_deref(), Some("a47"));
        assert_eq!(r.status.as_deref(), Some("async_launched"));
    }

    #[test]
    fn user_prompt_string_and_odd_tool_use_results() {
        let Parsed::User(u) =
            p(r#"{"type":"user","message":{"role":"user","content":"fix the bug"},"isMeta":true}"#)
        else {
            panic!()
        };
        assert!(matches!(&u.message.content, UserContent::Text(t) if t == "fix the bug"));
        assert!(u.is_meta);
        for tur in [
            r#""Error: denied""#,
            r#"[{"type":"text","text":"x"}]"#,
            "null",
            "42",
        ] {
            let line = format!(
                r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t","content":[{{"type":"text","text":"abc"}}]}}]}},"toolUseResult":{tur}}}"#
            );
            let Parsed::User(u) = p(&line) else {
                panic!("{tur}")
            };
            assert!(!u.tool_use_result.unwrap_or_default().is_async);
            let UserContent::Blocks(b) = &u.message.content else {
                panic!()
            };
            assert!(
                matches!(b[0], Block::ToolResult { content: 3, .. }),
                "{tur}"
            );
        }
    }

    #[test]
    fn system_compact_boundary() {
        let Parsed::System(s) = p(
            r#"{"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"auto","preTokens":967275,"postTokens":25651,"cumulativeDroppedTokens":1}}"#,
        ) else {
            panic!()
        };
        assert_eq!(s.subtype.as_deref(), Some("compact_boundary"));
        let m = s.compact_metadata.unwrap();
        assert_eq!(
            (m.pre_tokens, m.post_tokens, m.trigger.as_deref()),
            (Some(967275), Some(25651), Some("auto"))
        );
    }

    #[test]
    fn cost_state_and_titles() {
        let Parsed::CostState(c) = p(
            r#"{"type":"cost-state","sessionId":"s","totalCostUSD":476.39,"totalLinesAdded":1917,"totalLinesRemoved":5,"modelUsage":{"claude-haiku-5-5":{"inputTokens":1570,"outputTokens":26,"costUSD":0.01}}}"#,
        ) else {
            panic!()
        };
        assert_eq!(c.total_cost_usd, Some(476.39));
        assert_eq!(
            c.model_usage.unwrap()["claude-haiku-5-5"].input_tokens,
            1570
        );
        assert!(
            matches!(p(r#"{"type":"agent-name","agentName":"gs-api","sessionId":"s"}"#), Parsed::AgentName(n) if n == "gs-api")
        );
        assert!(
            matches!(p(r#"{"type":"custom-title","customTitle":"t"}"#), Parsed::CustomTitle(n) if n == "t")
        );
        assert!(
            matches!(p(r#"{"type":"ai-title","aiTitle":"a"}"#), Parsed::AiTitle(n) if n == "a")
        );
        assert!(
            matches!(p(r#"{"type":"last-prompt","lastPrompt":"841 merged"}"#), Parsed::LastPrompt(n) if n == "841 merged")
        );
    }

    #[test]
    fn unknown_type_is_ignored_and_type_found_when_not_first() {
        assert!(matches!(p(r#"{"type":"pr-link","x":1}"#), Parsed::Ignored(t) if t == "pr-link"));
        assert!(
            matches!(p(r#"{"a":1,"type":"something-new"}"#), Parsed::Ignored(t) if t == "something-new")
        );
    }

    #[test]
    fn malformed_lines_fail_soft() {
        let mut stats = ParseStats::default();
        let lines = vec![
            RawLine {
                offset: 0,
                bytes: br#"{"type":"agent-name","agentName":"x"}"#.to_vec(),
            },
            RawLine {
                offset: 40,
                bytes: br#"{"type":"assistant","message":{"#.to_vec(),
            },
            RawLine {
                offset: 80,
                bytes: br#"not json"#.to_vec(),
            },
            RawLine {
                offset: 90,
                bytes: br#"{"no":"type"}"#.to_vec(),
            },
            RawLine {
                offset: 100,
                bytes: b"".to_vec(),
            },
            RawLine {
                offset: 101,
                bytes: br#"{"type":"mode","mode":"normal"}"#.to_vec(),
            },
        ];
        let (good, bad) = parse_batch(lines, &mut stats);
        assert_eq!(good.len(), 2);
        assert_eq!(bad.len(), 3);
        assert_eq!(stats.errors, 3);
        assert_eq!(stats.parsed, 1);
        assert_eq!(stats.ignored_types["mode"], 1);
        assert!(
            stats
                .first_error
                .as_deref()
                .unwrap()
                .starts_with("offset 40")
        );
    }

    #[test]
    fn huge_ignored_line_is_skipped_without_decoding() {
        let mut line = br#"{"type":"api-request-blob","sessionId":"s","blob":""#.to_vec();
        line.extend(std::iter::repeat_n(b'x', 2 * 1024 * 1024));
        line.extend(br#""}"#);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            assert!(matches!(parse_line(&line, 0).unwrap(), Parsed::Ignored(_)));
        }
        assert!(
            start.elapsed().as_millis() < 500,
            "took {:?} for 100 lines",
            start.elapsed()
        );
    }

    #[test]
    fn huge_tool_result_parses() {
        let big = "y".repeat(2 * 1024 * 1024);
        let line = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t","content":"{big}"}}]}}}}"#
        );
        let Parsed::User(u) = p(&line) else { panic!() };
        let UserContent::Blocks(b) = &u.message.content else {
            panic!()
        };
        assert!(matches!(
            b[0],
            Block::ToolResult {
                content: 2097152,
                ..
            }
        ));
    }
}

/// Parses the last 16 MB of a real transcript named by `ANTFARM_PARSE_FILE`.
/// Run with `cargo test -p antfarm-core --release real_file -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn real_file() {
    use std::io::{Read, Seek, SeekFrom};
    let Some(path) = std::env::var_os("ANTFARM_PARSE_FILE") else {
        return;
    };
    let mut f = std::fs::File::open(path).unwrap();
    let len = f.metadata().unwrap().len();
    let start = len.saturating_sub(16 << 20);
    f.seek(SeekFrom::Start(start)).unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    let first_nl = if start > 0 {
        memchr::memchr(b'\n', &buf)
            .map(|i| i + 1)
            .unwrap_or(buf.len())
    } else {
        0
    };
    let mut offset = start + first_nl as u64;
    let mut raw = Vec::new();
    for line in buf[first_nl..].split(|b| *b == b'\n') {
        raw.push(RawLine {
            offset,
            bytes: line.to_vec(),
        });
        offset += line.len() as u64 + 1;
    }
    let mut stats = ParseStats::default();
    let t = std::time::Instant::now();
    let (good, bad) = parse_batch(raw, &mut stats);
    println!(
        "{} good, {} bad in {:?}",
        good.len(),
        bad.len(),
        t.elapsed()
    );
    println!("{stats:#?}");
    for e in bad.iter().take(5) {
        println!("{e:?}");
    }
}
