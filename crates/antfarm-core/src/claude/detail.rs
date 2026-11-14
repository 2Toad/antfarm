//! Reads a call's input and result back from its transcript, by byte offset.
//!
//! The model keeps inputs only for recent calls and results for none. The call screen
//! asks for them here. A front end can call this on every redraw: the last few lines
//! read stay parsed in a small cache.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::model::LineRefs;
use crate::tail::TailConfig;

/// Longest result text returned, in chars.
pub const RESULT_CHARS: usize = 20_000;
/// Lines kept parsed.
const CACHE: usize = 8;

/// A call's result as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultText {
    pub text: String,
    /// The text was longer than [`RESULT_CHARS`] and was cut.
    pub cut: bool,
    pub is_error: bool,
}

type Entry = ((PathBuf, u64), Option<Arc<Value>>);

static LINES: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());

/// The input of call `id` from the line at `lines.call`.
pub fn read_input(path: &Path, lines: LineRefs, id: &str) -> Option<Value> {
    let line = line_at(path, lines.call?)?;
    blocks(&line, "message")?
        .iter()
        .find(|b| {
            b.get("type").and_then(Value::as_str) == Some("tool_use")
                && b.get("id").and_then(Value::as_str) == Some(id)
        })
        .and_then(|b| b.get("input").cloned())
}

/// The result of call `id` from the line at `lines.result`.
pub fn read_result(path: &Path, lines: LineRefs, id: &str) -> Option<ResultText> {
    let line = line_at(path, lines.result?)?;
    let block = blocks(&line, "message")?.iter().find(|b| {
        b.get("type").and_then(Value::as_str) == Some("tool_result")
            && b.get("tool_use_id").and_then(Value::as_str) == Some(id)
    })?;
    let mut text = String::new();
    content_text(block.get("content").unwrap_or(&Value::Null), &mut text);
    let cut = text.chars().nth(RESULT_CHARS).is_some();
    if cut {
        let end = text
            .char_indices()
            .nth(RESULT_CHARS)
            .map_or(text.len(), |(i, _)| i);
        text.truncate(end);
    }
    Some(ResultText {
        text,
        cut,
        is_error: block
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn blocks<'a>(line: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    line.get(key)?.get("content")?.as_array()
}

fn content_text(v: &Value, out: &mut String) {
    match v {
        Value::String(s) => out.push_str(s),
        Value::Array(items) => {
            for item in items {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        out.push_str(item.get("text").and_then(Value::as_str).unwrap_or(""))
                    }
                    Some("image") => out.push_str("[image]"),
                    Some(other) => {
                        out.push('[');
                        out.push_str(other);
                        out.push(']');
                    }
                    None => content_text(item, out),
                }
            }
        }
        Value::Null => {}
        other => out.push_str(&other.to_string()),
    }
}

/// The line starting at `offset`, parsed, through the cache.
fn line_at(path: &Path, offset: u64) -> Option<Arc<Value>> {
    let key = (path.to_path_buf(), offset);
    if let Some((_, v)) = LINES.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return v.clone();
    }
    let v = read_line(path, offset)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .map(Arc::new);
    let mut cache = LINES.lock().unwrap();
    if cache.len() == CACHE {
        cache.pop_front();
    }
    cache.push_back((key, v.clone()));
    v
}

fn read_line(path: &Path, offset: u64) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    f.seek(SeekFrom::Start(offset)).ok()?;
    let max = TailConfig::default().max_line;
    let mut line = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            return Some(line);
        }
        if let Some(i) = memchr::memchr(b'\n', &buf[..n]) {
            line.extend_from_slice(&buf[..i]);
            return Some(line);
        }
        line.extend_from_slice(&buf[..n]);
        if line.len() > max {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_input_and_result_by_offset() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la"}}]}}"#;
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"denied"},{"type":"image","source":{}}]}]}}"#;
        std::fs::write(&p, format!("{call}\n{result}\n")).unwrap();
        let lines = LineRefs {
            call: Some(0),
            result: Some(call.len() as u64 + 1),
        };
        assert_eq!(read_input(&p, lines, "t1").unwrap()["command"], "ls -la");
        let r = read_result(&p, lines, "t1").unwrap();
        assert_eq!(r.text, "denied\n[image]");
        assert!(r.is_error && !r.cut);
        assert!(read_input(&p, lines, "other").is_none());
        assert!(read_result(&p, LineRefs::default(), "t1").is_none());
    }

    #[test]
    fn a_long_result_is_cut() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let long = "é".repeat(RESULT_CHARS + 5);
        let result = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"{long}"}}]}}}}"#
        );
        std::fs::write(&p, format!("{result}\n")).unwrap();
        let r = read_result(
            &p,
            LineRefs {
                call: None,
                result: Some(0),
            },
            "t1",
        )
        .unwrap();
        assert!(r.cut);
        assert_eq!(r.text.chars().count(), RESULT_CHARS);
    }
}
