//! Deterministic, shape-preserving anonymization of Claude Code JSON.
//!
//! Every string that could carry user data is replaced by text of the same character
//! length: letters stay letters of the same case, digits stay digits, a small set of
//! punctuation and all ASCII whitespace stay put, and everything else (including
//! non-ASCII) becomes an ASCII letter. Strings are split on `/` and each segment is
//! mapped on its own, keyed by its own content, so the same path segment or token maps
//! to the same output wherever it appears, and file extensions survive. The keystream
//! comes from `blake3` keyed with a constant, so re-running on the same input gives
//! byte-identical output.
//!
//! What is kept verbatim is an allowlist of structural keys ([`KEEP_VALUES`] and
//! [`KEEP_WHOLE`]) plus all numbers, booleans, and nulls. Everything else is scrubbed.
//! Inside subtrees that are entirely user data (`tool_use.input`, `tool_result.content`,
//! `toolUseResult`, workflow `args`, `result`, and `logs`) object keys are scrubbed too,
//! unless they are well-known tool parameter or result field names ([`DATA_KEYS`]).

use crate::json::Json;

/// Fixed key for the keyed hash. Determinism matters more than secrecy here: the output
/// cannot be reversed, but a guessed token can be confirmed by anyone with this key.
pub const KEY: [u8; 32] = *b"antfarm-scrub fixture key v1.0!!";

/// Replaces base64 image payloads. Sixteen bytes.
pub const IMAGE_PLACEHOLDER: &str = "image-data-cut!!";

/// Keys whose scalar values are structural and stay verbatim everywhere outside user data
/// subtrees. `agentType` is kept only on a built-in sub-agent meta.
pub const KEEP_VALUES: &[&str] = &[
    "type",
    "subtype",
    "id",
    "uuid",
    "parentUuid",
    "logicalParentUuid",
    "tool_use_id",
    "toolUseId",
    "agentId",
    "parentAgentId",
    "sessionId",
    "promptId",
    "requestId",
    "leafUuid",
    "sourceToolAssistantUUID",
    "sourceToolUseID",
    "model",
    "requestedModel",
    "resolvedModel",
    "effort",
    "role",
    "stop_reason",
    "status",
    "state",
    "timestamp",
    "version",
    "entrypoint",
    "userType",
    "is_error",
    "isSidechain",
    "isMeta",
    "isCompactSummary",
    "isAsync",
    "requestShape",
    "spawnDepth",
    "finished",
    "runId",
    "taskId",
    "trigger",
    "operation",
    "kind",
    "decision",
    "permissionMode",
    "waitingFor",
    "media_type",
];

/// Objects kept whole: token counts and compaction numbers.
pub const KEEP_WHOLE: &[&str] = &["usage", "modelUsage", "compactMetadata"];

/// Object keys kept as-is inside user data subtrees: built-in tool parameters, tool
/// result fields, and workflow argument and result fields. Any other key there is
/// treated as user data and mapped like a string.
pub const DATA_KEYS: &[&str] = &[
    // Built-in tool parameters.
    "command",
    "description",
    "file_path",
    "notebook_path",
    "old_string",
    "new_string",
    "content",
    "pattern",
    "path",
    "glob",
    "url",
    "query",
    "prompt",
    "subagent_type",
    "questions",
    "question",
    "options",
    "label",
    "header",
    "multiSelect",
    "skill",
    "args",
    "timeout",
    "run_in_background",
    "limit",
    "offset",
    "allow_large",
    "model",
    "name",
    "to",
    "summary",
    "message",
    "recipient",
    "recipient_kind",
    "delaySeconds",
    "reason",
    "noop",
    "scriptPath",
    "edits",
    "replace_all",
    "output_mode",
    "task_id",
    "agent_id",
    "max_results",
    "isolation",
    "effort",
    "head_limit",
    "multiline",
    "cell_id",
    "cell_type",
    "edit_mode",
    "new_source",
    "pages",
    "-n",
    "-i",
    "-A",
    "-B",
    "-C",
    "dangerouslyDisableSandbox",
    "answers",
    "text",
    "source",
    "data",
    "baseUrl",
    // Tool result fields.
    "stdout",
    "stderr",
    "interrupted",
    "isImage",
    "noOutputExpected",
    "bashEditDiff",
    "files",
    "hunks",
    "lines",
    "oldStart",
    "oldLines",
    "newStart",
    "newLines",
    "filePath",
    "created",
    "deleted",
    "changedFiles",
    "moreFiles",
    "shared",
    "wasClamped",
    "scheduledFor",
    "clampedDelaySeconds",
    "success",
    "pin",
    "ref",
    "revivedAgentId",
    "resumedAgentId",
    "workflowName",
    "transcriptDir",
    "taskType",
    "outputFile",
    "canReadOutputFile",
    "canContinueAgent",
    "gitOperation",
    "pr",
    "number",
    "action",
    "push",
    "branch",
    "structuredPatch",
    "userModified",
    "originalFile",
    "backgroundTaskId",
    "background",
    "staleReadFileStateHint",
    "memdirStamped",
    "replaceAll",
    "persistedOutputSize",
    "persistedOutputPath",
    "oldString",
    "newString",
    "contentNotInModelContext",
    "commandName",
    "backgroundCwdHint",
    "filenames",
    "numFiles",
    "numLines",
    "durationMs",
    "totalDurationMs",
    "totalTokens",
    "totalToolUseCount",
    "file",
    "startLine",
    "totalLines",
    "result",
    "error",
    "mode",
    "title",
    "bytes",
    "results",
    "truncated",
    // Workflow args and result.
    "base",
    "worktree",
    "narrative",
    "findings",
    "needsVerify",
    "lenses",
];

/// String values kept at the top level of `toolUseResult`.
const RESULT_KEEP_VALUES: &[&str] = &[
    "type",
    "status",
    "agentId",
    "resolvedModel",
    "runId",
    "taskId",
    "taskType",
];

/// Tags of a `<task-notification>` whose markup survives; everything between other
/// angle brackets is treated as text.
const NOTIFICATION_TAGS: &[&str] = &[
    "task-notification",
    "task-id",
    "tool-use-id",
    "output-file",
    "status",
    "summary",
    "result",
    "usage",
    "total_tokens",
    "tool_uses",
    "subagent_tokens",
    "duration_ms",
    "diagnostics",
    "agents_skipped",
    "agents_error",
    "failures",
    "note",
];

/// Tags directly inside a `<task-notification>` whose text is kept verbatim.
const NOTIFICATION_KEPT: &[&str] = &["task-id", "status", "tool-use-id"];

const KEPT_PUNCT: &[char] = &[
    '/', '.', '-', '_', ':', ',', '"', '\'', '{', '}', '[', ']', '(', ')', '<', '>', '=', '\\',
];

/// Which rules apply inside a user data subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Data {
    /// `tool_use.input`, workflow `args`/`result`/`logs`: no string value survives.
    Input,
    /// `tool_result.content` blocks: `type` and `media_type` survive.
    Content,
    /// Top level of `toolUseResult`: [`RESULT_KEEP_VALUES`] survive.
    Result,
}

fn keystream(seed: &[u8], n: usize) -> Vec<u8> {
    let mut h = blake3::Hasher::new_keyed(&KEY);
    h.update(seed);
    let mut out = vec![0u8; n];
    h.finalize_xof().fill(&mut out);
    out
}

/// Maps one token: same character count, letters to letters of the same case, digits to
/// digits, kept punctuation and ASCII whitespace unchanged, anything else to a lowercase
/// letter. The letter after a backslash is kept when it is a JSON escape letter so
/// escapes inside JSON-in-strings stay valid.
pub fn map_segment(seg: &str) -> String {
    let n = seg.chars().count();
    if n == 0 {
        return String::new();
    }
    let ks = keystream(seg.as_bytes(), n);
    let mut out = String::with_capacity(n);
    let mut prev = '\0';
    for (i, c) in seg.chars().enumerate() {
        let k = ks[i];
        let mapped = if c.is_ascii_uppercase() {
            (b'A' + k % 26) as char
        } else if c.is_ascii_lowercase() {
            if prev == '\\' && matches!(c, 'n' | 't' | 'r' | 'u' | 'b' | 'f') {
                c
            } else {
                (b'a' + k % 26) as char
            }
        } else if c.is_ascii_digit() {
            (b'0' + k % 10) as char
        } else if c.is_ascii_whitespace() || KEPT_PUNCT.contains(&c) {
            c
        } else {
            (b'a' + k % 26) as char
        };
        out.push(mapped);
        prev = c;
    }
    out
}

/// Maps a path segment, keeping a short alphanumeric file extension.
fn map_path_segment(seg: &str) -> String {
    if let Some(dot) = seg.rfind('.') {
        let (stem, ext) = (&seg[..dot], &seg[dot + 1..]);
        let ext_ok = !ext.is_empty()
            && ext.len() <= 6
            && ext.bytes().all(|b| b.is_ascii_alphanumeric())
            && ext.bytes().any(|b| b.is_ascii_alphabetic());
        if !stem.is_empty()
            && ext_ok
            && seg.len() <= 255
            && !seg.bytes().any(|b| b.is_ascii_whitespace())
        {
            return format!("{}.{ext}", map_segment(stem));
        }
    }
    map_segment(seg)
}

fn scrub_plain(s: &str) -> String {
    if !s.contains('/') {
        return map_path_segment(s);
    }
    let parts: Vec<String> = s.split('/').map(map_path_segment).collect();
    parts.join("/")
}

/// Scrubs a `<task-notification>` string: known tag markup is kept, the text of
/// `task-id`, `status`, and `tool-use-id` directly inside the notification is kept,
/// everything else is mapped.
fn scrub_notification(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut depth = 0usize;
    let mut keep = false;
    let flush = |text: &str, keep: bool, out: &mut String| {
        if keep {
            out.push_str(text);
        } else {
            out.push_str(&scrub_plain(text));
        }
    };
    loop {
        let Some(lt) = rest.find('<') else {
            flush(rest, keep, &mut out);
            break;
        };
        let after = &rest[lt..];
        if let Some(gt) = after.find('>') {
            let tag = &after[1..gt];
            let (closing, name) = match tag.strip_prefix('/') {
                Some(n) => (true, n),
                None => (false, tag),
            };
            if NOTIFICATION_TAGS.contains(&name) {
                flush(&rest[..lt], keep, &mut out);
                out.push_str(&after[..=gt]);
                if closing {
                    depth = depth.saturating_sub(1);
                    keep = false;
                } else {
                    depth += 1;
                    keep = depth == 2 && NOTIFICATION_KEPT.contains(&name);
                }
                rest = &after[gt + 1..];
                continue;
            }
        }
        flush(&rest[..=lt], keep, &mut out);
        rest = &rest[lt + 1..];
    }
    out
}

/// Scrubs any user-visible string.
pub fn scrub_text(s: &str) -> String {
    if s == IMAGE_PLACEHOLDER {
        return s.to_owned();
    }
    if s.contains("<task-notification>") {
        scrub_notification(s)
    } else {
        scrub_plain(s)
    }
}

/// Built-in tool names pass through. MCP names keep their `mcp__` prefix and have the
/// server and tool segments mapped.
pub fn map_tool_name(name: &str) -> String {
    if !name.contains("__") {
        return name.to_owned();
    }
    let mut parts = name.split("__");
    let first = parts.next().unwrap_or_default();
    let mut out = if first == "mcp" {
        first.to_owned()
    } else {
        map_segment(first)
    };
    for p in parts {
        out.push_str("__");
        out.push_str(&map_segment(p));
    }
    out
}

/// Replaces image payloads in place: `source.data` of any `image` block and any `base64`
/// field, at any depth.
pub fn replace_image_data(v: &mut Json) {
    match v {
        Json::Array(a) => a.iter_mut().for_each(replace_image_data),
        Json::Object(o) => {
            let is_image = o
                .iter()
                .any(|(k, v)| k == "type" && v.as_str() == Some("image"));
            for (k, val) in o.iter_mut() {
                if k == "base64" && matches!(val, Json::String(_)) {
                    *val = Json::String(IMAGE_PLACEHOLDER.to_owned());
                } else if is_image && k == "source" {
                    if let Some(d) = val.get_mut("data") {
                        *d = Json::String(IMAGE_PLACEHOLDER.to_owned());
                    }
                    replace_image_data(val);
                } else {
                    replace_image_data(val);
                }
            }
        }
        _ => {}
    }
}

fn kept_key(k: &str) -> bool {
    KEEP_VALUES.contains(&k)
}

/// Schema-shaped JSON: keys are Claude Code's own field names and stay; string values are
/// scrubbed unless their key is structural.
fn scrub_schema(v: &mut Json) {
    match v {
        Json::String(s) => *s = scrub_text(s),
        Json::Array(a) => a.iter_mut().for_each(scrub_schema),
        Json::Object(o) => {
            let built_in = o
                .iter()
                .any(|(k, v)| k == "isBuiltIn" && v.as_bool() == Some(true));
            for (k, val) in o.iter_mut() {
                if KEEP_WHOLE.contains(&k.as_str()) {
                    continue;
                }
                if val.is_scalar() && (kept_key(k) || (k == "agentType" && built_in)) {
                    continue;
                }
                if k == "wireToolInputs" {
                    scrub_data(val, Data::Input, 0);
                } else {
                    scrub_schema(val);
                }
            }
        }
        _ => {}
    }
}

/// User data JSON: keys that are not well-known names are mapped too.
fn scrub_data(v: &mut Json, mode: Data, depth: usize) {
    match v {
        Json::String(s) => *s = scrub_text(s),
        Json::Array(a) => a.iter_mut().for_each(|x| scrub_data(x, mode, depth + 1)),
        Json::Object(o) => {
            for (k, val) in o.iter_mut() {
                let keep_value = match val {
                    Json::String(_) => match mode {
                        Data::Input => false,
                        Data::Content => k == "type" || k == "media_type",
                        Data::Result => {
                            k == "media_type"
                                || (depth == 0 && RESULT_KEEP_VALUES.contains(&k.as_str()))
                                || (depth > 0 && k == "type")
                        }
                    },
                    _ => false,
                };
                if !keep_value {
                    scrub_data(val, mode, depth + 1);
                }
                if !(DATA_KEYS.contains(&k.as_str()) || kept_key(k)) {
                    *k = map_segment(k);
                }
            }
        }
        _ => {}
    }
}

fn scrub_block(b: &mut Json) {
    let kind = b
        .get("type")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_owned();
    let Json::Object(o) = b else {
        return scrub_schema(b);
    };
    match kind.as_str() {
        "tool_use" => {
            for (k, val) in o.iter_mut() {
                match k.as_str() {
                    "name" => {
                        if let Json::String(s) = val {
                            *s = map_tool_name(s);
                        }
                    }
                    "input" => scrub_data(val, Data::Input, 0),
                    "id" | "type" => {}
                    _ => scrub_schema(val),
                }
            }
        }
        "tool_result" => {
            for (k, val) in o.iter_mut() {
                match k.as_str() {
                    "content" => match val {
                        Json::String(s) => *s = scrub_text(s),
                        other => scrub_data(other, Data::Content, 0),
                    },
                    k if val.is_scalar() && kept_key(k) => {}
                    _ => scrub_schema(val),
                }
            }
        }
        _ => scrub_schema(b),
    }
}

fn scrub_message(m: &mut Json) {
    let Json::Object(o) = m else {
        return scrub_schema(m);
    };
    for (k, val) in o.iter_mut() {
        match k.as_str() {
            "content" => match val {
                Json::String(s) => *s = scrub_text(s),
                Json::Array(a) => a.iter_mut().for_each(scrub_block),
                other => scrub_schema(other),
            },
            k if KEEP_WHOLE.contains(&k) => {}
            k if val.is_scalar() && kept_key(k) => {}
            _ => scrub_schema(val),
        }
    }
}

/// Scrubs one transcript line in place, dropping `drop_keys` first.
pub fn scrub_line(line: &mut Json, drop_keys: &[String]) {
    for k in drop_keys {
        line.remove(k);
    }
    replace_image_data(line);
    let Json::Object(o) = line else {
        return scrub_schema(line);
    };
    for (k, val) in o.iter_mut() {
        match k.as_str() {
            "message" => scrub_message(val),
            "toolUseResult" => scrub_data(val, Data::Result, 0),
            k if KEEP_WHOLE.contains(&k) => {}
            k if val.is_scalar() && kept_key(k) => {}
            _ => scrub_schema(val),
        }
    }
}

/// Scrubs a sub-agent meta file or a registry entry in place.
pub fn scrub_meta(meta: &mut Json) {
    scrub_schema(meta);
}

/// Scrubs a workflow file in place. `args`, `result`, and `logs` are user data; the rest
/// is schema with the usual structural keys kept.
pub fn scrub_workflow(wf: &mut Json) {
    replace_image_data(wf);
    let Json::Object(o) = wf else {
        return scrub_schema(wf);
    };
    for (k, val) in o.iter_mut() {
        match k.as_str() {
            "args" | "result" | "logs" => scrub_data(val, Data::Input, 0),
            k if val.is_scalar() && kept_key(k) => {}
            _ => scrub_schema(val),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scrubbed(line: &str) -> Json {
        let mut j = Json::parse(line.as_bytes()).unwrap();
        scrub_line(&mut j, &[]);
        j
    }

    #[test]
    fn char_classes_are_preserved() {
        let src = "Hello, World 42! café/naïve_x-y.z: \"q\" {a} [b] (c) <d> e=f\\n\tz\n";
        let out = map_segment(src);
        assert_eq!(out.chars().count(), src.chars().count());
        assert!(out.is_ascii());
        for (a, b) in src.chars().zip(out.chars()) {
            if a.is_ascii_uppercase() {
                assert!(b.is_ascii_uppercase(), "{a} -> {b}");
            } else if a.is_ascii_lowercase() {
                assert!(b.is_ascii_lowercase(), "{a} -> {b}");
            } else if a.is_ascii_digit() {
                assert!(b.is_ascii_digit(), "{a} -> {b}");
            } else if a.is_ascii_whitespace() || KEPT_PUNCT.contains(&a) {
                assert_eq!(a, b);
            } else {
                assert!(b.is_ascii_lowercase(), "{a} -> {b}");
            }
        }
        assert_ne!(out, src);
        // The escape letter after a backslash survives.
        assert!(out.contains("\\n"));
    }

    #[test]
    fn path_segments_map_consistently_and_keep_extensions() {
        let a = scrub_text("/Users/jason/proj/src/main.rs");
        let b = scrub_text("/Users/jason/other/main.rs");
        let sa: Vec<&str> = a.split('/').collect();
        let sb: Vec<&str> = b.split('/').collect();
        assert_eq!(sa[1], sb[1]);
        assert_eq!(sa[2], sb[2]);
        assert_eq!(sa[5], sb[4]);
        assert!(sa[5].ends_with(".rs"));
        assert_ne!(sa[5], "main.rs");
        assert_eq!(scrub_text("main.rs"), sa[5]);
        assert_eq!(scrub_text("jason"), sa[2]);
        assert_ne!(scrub_text("Users"), "Users");
        // Dot files and numeric suffixes have no extension to keep.
        assert!(!scrub_text(".gitignore").ends_with("gitignore"));
        assert!(!scrub_text("2.1.314").ends_with("314"));
    }

    #[test]
    fn scrubbing_is_deterministic() {
        let line = r#"{"parentUuid":"p","type":"user","cwd":"/Users/me/x","message":{"role":"user","content":"please fix src/lib.rs"},"toolUseResult":{"stdout":"ok 1\n","answers":{"Which?":"B"}}}"#;
        let a = scrubbed(line).to_vec();
        let b = scrubbed(line).to_vec();
        assert_eq!(a, b);
        // A fixed mapping, so an accidental key change is noticed.
        assert_eq!(scrub_text("hello"), "uvvfw");
    }

    #[test]
    fn keep_list_keys_are_untouched_and_the_rest_are_not() {
        let line = r#"{"parentUuid":"u0","isSidechain":false,"userType":"external","cwd":"/Users/me/p","sessionId":"s-1","version":"2.1.314","gitBranch":"main","slug":"x-y","type":"assistant","uuid":"u1","timestamp":"2026-11-02T00:21:58.124Z","requestedModel":"opus","effort":"high","message":{"id":"msg_1","model":"claude-opus-5-5","role":"assistant","stop_reason":"tool_use","content":[{"type":"thinking","thinking":"secret","signature":"sig"},{"type":"text","text":"hello there"},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"cat /etc/x","description":"read"}}],"usage":{"input_tokens":2,"cache_read_input_tokens":24,"output_tokens":245,"output_tokens_details":{"thinking_tokens":58}}},"permissionDecision":{"decision":"allow","source":"rule x"}}"#;
        let j = scrubbed(line);
        for (k, want) in [
            ("parentUuid", "u0"),
            ("userType", "external"),
            ("sessionId", "s-1"),
            ("version", "2.1.314"),
            ("type", "assistant"),
            ("uuid", "u1"),
            ("timestamp", "2026-11-02T00:21:58.124Z"),
            ("requestedModel", "opus"),
            ("effort", "high"),
        ] {
            assert_eq!(j.get(k).and_then(Json::as_str), Some(want), "{k}");
        }
        assert_eq!(j.get("isSidechain").and_then(Json::as_bool), Some(false));
        for k in ["cwd", "gitBranch", "slug"] {
            let v = j.get(k).and_then(Json::as_str).unwrap();
            assert_ne!(v, "", "{k}");
            assert!(!v.contains("me") && !v.contains("main"), "{k}: {v}");
        }
        assert!(j.get("cwd").unwrap().as_str().unwrap().starts_with('/'));
        let m = j.get("message").unwrap();
        assert_eq!(m.get("id").and_then(Json::as_str), Some("msg_1"));
        assert_eq!(
            m.get("model").and_then(Json::as_str),
            Some("claude-opus-5-5")
        );
        assert_eq!(
            m.get("stop_reason").and_then(Json::as_str),
            Some("tool_use")
        );
        assert_eq!(
            m.get("usage").unwrap().to_vec(),
            br#"{"input_tokens":2,"cache_read_input_tokens":24,"output_tokens":245,"output_tokens_details":{"thinking_tokens":58}}"#
        );
        let blocks = m.get("content").unwrap().as_array().unwrap();
        assert_eq!(
            blocks[0].get("type").and_then(Json::as_str),
            Some("thinking")
        );
        assert_ne!(
            blocks[0].get("thinking").and_then(Json::as_str),
            Some("secret")
        );
        assert_eq!(
            blocks[0].get("thinking").unwrap().as_str().unwrap().len(),
            6
        );
        assert_ne!(
            blocks[0].get("signature").and_then(Json::as_str),
            Some("sig")
        );
        assert_ne!(
            blocks[1].get("text").and_then(Json::as_str),
            Some("hello there")
        );
        assert_eq!(blocks[2].get("id").and_then(Json::as_str), Some("toolu_1"));
        assert_eq!(blocks[2].get("name").and_then(Json::as_str), Some("Bash"));
        let input = blocks[2].get("input").unwrap();
        let cmd = input.get("command").and_then(Json::as_str).unwrap();
        assert_eq!(cmd.len(), 10);
        assert!(!cmd.contains("etc"));
        assert_eq!(&cmd.as_bytes()[3..5], b" /");
        assert_ne!(
            input.get("description").and_then(Json::as_str),
            Some("read")
        );
        let pd = j.get("permissionDecision").unwrap();
        assert_eq!(pd.get("decision").and_then(Json::as_str), Some("allow"));
        assert_ne!(pd.get("source").and_then(Json::as_str), Some("rule x"));
    }

    #[test]
    fn task_notification_keeps_tags_id_and_status() {
        let text = "<task-notification>\n<task-id>ab1220b7806d11040</task-id>\n<tool-use-id>toolu_01Abc</tool-use-id>\n<output-file>/tmp/x/agent.output</output-file>\n<status>completed</status>\n<summary>Agent \"review PR #123\" completed</summary>\n<result>Found <status>bad</status> things in /src/a.rs</result>\n</task-notification>";
        let out = scrub_text(text);
        assert_eq!(out.chars().count(), text.chars().count());
        assert!(out.contains("<task-id>ab1220b7806d11040</task-id>"));
        assert!(out.contains("<tool-use-id>toolu_01Abc</tool-use-id>"));
        assert!(out.contains("\n<status>completed</status>\n"));
        assert!(out.contains("<summary>") && out.contains("</summary>"));
        assert!(!out.contains("review PR"));
        assert!(!out.contains("agent.output"));
        assert!(out.contains("<output-file>/"));
        // A status tag nested inside the result body is not a kept value.
        assert!(!out.contains("<status>bad</status>"));
        assert!(!out.contains("a.rs"));
        assert!(out.ends_with("</task-notification>"));
    }

    #[test]
    fn mcp_tool_names_are_mapped_and_builtins_kept() {
        for name in [
            "Bash",
            "Read",
            "AskUserQuestion",
            "SendMessage",
            "SubagentHandback",
        ] {
            assert_eq!(map_tool_name(name), name);
        }
        let m = map_tool_name("mcp__greatsword__command");
        assert!(m.starts_with("mcp__"));
        assert_eq!(m.len(), "mcp__greatsword__command".len());
        assert!(!m.contains("greatsword") && !m.contains("command"));
        assert_eq!(m.matches("__").count(), 2);
        assert_eq!(m, map_tool_name("mcp__greatsword__command"));
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":"mcp__greatsword__connect","input":{"profile":"dev","baseUrl":"http://x"}}]}}"#;
        let j = scrubbed(line);
        let b = &j
            .get("message")
            .unwrap()
            .get("content")
            .unwrap()
            .as_array()
            .unwrap()[0];
        assert_eq!(
            b.get("name").and_then(Json::as_str),
            Some(&*m.replace(&map_segment("command"), &map_segment("connect"),))
        );
        let input = b.get("input").unwrap();
        // An unknown parameter key is mapped, a well-known one is kept.
        assert!(input.get("profile").is_none());
        assert!(input.get("baseUrl").is_some());
        assert_ne!(
            input.get("baseUrl").and_then(Json::as_str),
            Some("http://x")
        );
    }

    #[test]
    fn tool_use_result_shapes() {
        let s = scrubbed(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"Error: denied /x/y"}]},"toolUseResult":"Error: denied /x/y"}"#,
        );
        let r = s.get("toolUseResult").and_then(Json::as_str).unwrap();
        assert_eq!(r.len(), "Error: denied /x/y".len());
        assert!(!r.contains("denied"));
        let blocks = s
            .get("message")
            .unwrap()
            .get("content")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            blocks[0].get("tool_use_id").and_then(Json::as_str),
            Some("t1")
        );
        assert_eq!(
            blocks[0].get("is_error").and_then(Json::as_bool),
            Some(true)
        );
        assert_eq!(blocks[0].get("content").and_then(Json::as_str), Some(r));

        let l = scrubbed(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"hello world"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAABBBBCCCCDDDDEEEE"}}]}]},"toolUseResult":[{"type":"text","text":"hello world"}]}"#,
        );
        let r = l.get("toolUseResult").unwrap().as_array().unwrap();
        assert_eq!(r[0].get("type").and_then(Json::as_str), Some("text"));
        assert_eq!(r[0].get("text").unwrap().as_str().unwrap().len(), 11);
        assert_ne!(r[0].get("text").and_then(Json::as_str), Some("hello world"));
        let c = l
            .get("message")
            .unwrap()
            .get("content")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .get("content")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            c[1].get("source")
                .unwrap()
                .get("media_type")
                .and_then(Json::as_str),
            Some("image/png")
        );
        assert_eq!(
            c[1].get("source")
                .unwrap()
                .get("data")
                .and_then(Json::as_str),
            Some(IMAGE_PLACEHOLDER)
        );

        let o = scrubbed(
            r#"{"type":"user","message":{"content":[]},"toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"a47","description":"review the thing","resolvedModel":"sonnet","prompt":"do x","outputFile":"/tmp/t/a47.output","answers":{"Which palette?":"Warm"},"questions":[{"question":"Which palette?","header":"Palette","options":[{"label":"Warm","description":"d"}]}]}}"#,
        );
        let r = o.get("toolUseResult").unwrap();
        assert_eq!(r.get("isAsync").and_then(Json::as_bool), Some(true));
        assert_eq!(
            r.get("status").and_then(Json::as_str),
            Some("async_launched")
        );
        assert_eq!(r.get("agentId").and_then(Json::as_str), Some("a47"));
        assert_eq!(
            r.get("resolvedModel").and_then(Json::as_str),
            Some("sonnet")
        );
        assert_ne!(
            r.get("description").and_then(Json::as_str),
            Some("review the thing")
        );
        assert!(
            r.get("outputFile")
                .unwrap()
                .as_str()
                .unwrap()
                .ends_with(".output")
        );
        let answers = r.get("answers").unwrap();
        assert!(answers.get("Which palette?").is_none());
        let Json::Object(entries) = answers else {
            panic!()
        };
        assert_eq!(entries[0].0, map_segment("Which palette?"));
        assert_eq!(
            o.get("toolUseResult")
                .unwrap()
                .get("questions")
                .unwrap()
                .as_array()
                .unwrap()[0]
                .get("question")
                .and_then(Json::as_str),
            Some(entries[0].0.as_str())
        );
        assert_eq!(entries[0].1.as_str().unwrap().len(), 4);
    }

    #[test]
    fn a_forked_skill_result_keeps_its_shape() {
        let j = scrubbed(
            r#"{"type":"user","message":{"content":[]},"toolUseResult":{"success":true,"commandName":"secret-skill","status":"forked","background":true,"agentId":"a94f","result":"private words"}}"#,
        );
        let r = j.get("toolUseResult").unwrap();
        assert_eq!(r.get("status").and_then(Json::as_str), Some("forked"));
        assert_eq!(r.get("background").and_then(Json::as_bool), Some(true));
        assert_eq!(r.get("agentId").and_then(Json::as_str), Some("a94f"));
        assert_ne!(
            r.get("commandName").and_then(Json::as_str),
            Some("secret-skill")
        );
        assert_ne!(
            r.get("result").and_then(Json::as_str),
            Some("private words")
        );
    }

    #[test]
    fn meta_registry_and_workflow() {
        let mut built_in = Json::parse(br#"{"agentType":"general-purpose","isBuiltIn":true,"description":"look at x","toolUseId":"toolu_1","spawnDepth":2,"parentAgentId":"a1","requestShape":"foreground","model":"sonnet","finished":true}"#).unwrap();
        scrub_meta(&mut built_in);
        assert_eq!(
            built_in.get("agentType").and_then(Json::as_str),
            Some("general-purpose")
        );
        assert_ne!(
            built_in.get("description").and_then(Json::as_str),
            Some("look at x")
        );
        assert_eq!(
            built_in.get("toolUseId").and_then(Json::as_str),
            Some("toolu_1")
        );
        assert_eq!(
            built_in.get("parentAgentId").and_then(Json::as_str),
            Some("a1")
        );
        assert_eq!(built_in.get("spawnDepth").and_then(Json::as_u64), Some(2));
        let mut custom =
            Json::parse(br#"{"agentType":"my-private-agent","description":"d"}"#).unwrap();
        scrub_meta(&mut custom);
        assert_ne!(
            custom.get("agentType").and_then(Json::as_str),
            Some("my-private-agent")
        );

        let mut reg = Json::parse(br#"{"pid":748,"sessionId":"s","cwd":"/Users/me/p","name":"p","status":"waiting","waitingFor":"permission prompt","messagingSocketPath":"/tmp/cc-socks/748.sock","kind":"interactive","peerFeatures":["notify_idle"]}"#).unwrap();
        scrub_meta(&mut reg);
        assert_eq!(reg.get("pid").and_then(Json::as_u64), Some(748));
        assert_eq!(reg.get("status").and_then(Json::as_str), Some("waiting"));
        assert_eq!(
            reg.get("waitingFor").and_then(Json::as_str),
            Some("permission prompt")
        );
        assert_ne!(reg.get("cwd").and_then(Json::as_str), Some("/Users/me/p"));
        assert!(
            reg.get("messagingSocketPath")
                .unwrap()
                .as_str()
                .unwrap()
                .ends_with(".sock")
        );
        assert_ne!(
            reg.get("peerFeatures").unwrap().as_array().unwrap()[0].as_str(),
            Some("notify_idle")
        );

        let mut wf = Json::parse(br#"{"runId":"wf_1","taskId":"t1","workflowName":"review","status":"completed","script":"const x = 1;","args":{"base":"main","secretKey":"v"},"result":{"narrative":"all good"},"logs":["line one"],"phases":[{"title":"Claims","detail":"read"}],"workflowProgress":[{"type":"workflow_agent","agentId":"a1","state":"done","label":"extract","tokens":5}]}"#).unwrap();
        scrub_workflow(&mut wf);
        assert_eq!(wf.get("runId").and_then(Json::as_str), Some("wf_1"));
        assert_eq!(wf.get("taskId").and_then(Json::as_str), Some("t1"));
        assert_eq!(wf.get("status").and_then(Json::as_str), Some("completed"));
        assert_ne!(
            wf.get("workflowName").and_then(Json::as_str),
            Some("review")
        );
        assert_ne!(
            wf.get("script").and_then(Json::as_str),
            Some("const x = 1;")
        );
        let args = wf.get("args").unwrap();
        assert!(args.get("base").is_some() && args.get("secretKey").is_none());
        assert_ne!(args.get("base").and_then(Json::as_str), Some("main"));
        assert_ne!(
            wf.get("logs").unwrap().as_array().unwrap()[0].as_str(),
            Some("line one")
        );
        let p = &wf.get("workflowProgress").unwrap().as_array().unwrap()[0];
        assert_eq!(p.get("agentId").and_then(Json::as_str), Some("a1"));
        assert_eq!(p.get("state").and_then(Json::as_str), Some("done"));
        assert_eq!(p.get("tokens").and_then(Json::as_u64), Some(5));
        assert_ne!(p.get("label").and_then(Json::as_str), Some("extract"));
        assert_ne!(
            wf.get("phases").unwrap().as_array().unwrap()[0]
                .get("title")
                .and_then(Json::as_str),
            Some("Claims")
        );
    }

    #[test]
    fn dropped_keys_and_images_in_message_content() {
        let mut j = Json::parse(br#"{"type":"user","wireToolInputs":{"toolu_1":{"command":"x"}},"message":{"content":[{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"ZZZZZZZZZZZZZZZZZZZZZZZZ"}}]}}"#).unwrap();
        scrub_line(&mut j, &["wireToolInputs".to_owned()]);
        assert!(j.get("wireToolInputs").is_none());
        let b = &j
            .get("message")
            .unwrap()
            .get("content")
            .unwrap()
            .as_array()
            .unwrap()[0];
        assert_eq!(
            b.get("source").unwrap().get("data").and_then(Json::as_str),
            Some(IMAGE_PLACEHOLDER)
        );
        assert_eq!(
            b.get("source")
                .unwrap()
                .get("media_type")
                .and_then(Json::as_str),
            Some("image/jpeg")
        );
    }
}
