//! Claude-specific line handling: turns parsed transcript lines into model changes.
//!
//! The engine owns the model and calls in here once per transcript batch. Pairing state
//! that is not part of the serialized model, such as which agent owns an open call and
//! what each transcript has seen so far, lives in [`SessionSide`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::line::{AssistantLine, Block, Envelope, SystemLine, UserContent, UserLine};
use super::parse::{ParseError, Parsed, ParsedLine};
use crate::engine::Emitter;
use crate::event::Event;
use crate::input::AgentRef;
use crate::model::{
    Activity, Agent, AgentId, FileTouch, ParseStats, ResultStatus, Session, Time, ToolCall,
    ToolCallId, ToolCategory,
};

/// Longest tool call summary, in chars.
pub const SUMMARY_CHARS: usize = 120;
/// Longest session task (first prompt), in chars.
pub const TASK_CHARS: usize = 200;
/// How many touched files a session remembers, most recent last.
pub const FILES_TOUCHED_CAP: usize = 200;

/// Per-session state the engine keeps beside the model.
#[derive(Debug, Default)]
pub(crate) struct SessionSide {
    /// Which agent holds each open call, so a result written to another transcript
    /// still closes it.
    pub call_index: HashMap<ToolCallId, AgentId>,
    /// One entry per transcript, keyed by the agent it belongs to.
    pub transcripts: HashMap<AgentId, TranscriptState>,
}

#[derive(Debug, Default)]
pub(crate) struct TranscriptState {
    pub truncated_before: Option<u64>,
    /// A `tool_use` has been read from this transcript since attach or truncation.
    pub seen_tool_use: bool,
    /// The assistant message whose text block was already recorded. Step 6 reuses the
    /// same idea for usage dedupe by `message.id`.
    pub text_message: Option<String>,
}

/// Which agent a transcript belongs to, creating a placeholder sub-agent when a
/// transcript shows up before anything else has named it.
pub(crate) fn resolve_agent(
    session: &mut Session,
    agent: &AgentRef,
    at: Option<Time>,
    em: &mut Emitter,
) -> AgentId {
    match agent {
        AgentRef::Root => session.root.clone(),
        AgentRef::Sub(id) => {
            if !session.agents.contains_key(id) {
                let root = session.root.clone();
                let mut a = Agent::new(id.clone(), "unknown");
                a.parent = Some(root.clone());
                a.depth = 1;
                session.agents.insert(id.clone(), a);
                if let Some(r) = session.agents.get_mut(&root) {
                    r.children.push(id.clone());
                }
                em.emit(
                    &session.id,
                    at,
                    Event::AgentSpawned {
                        agent: id.clone(),
                        parent: Some(root),
                        kind: "unknown".into(),
                        description: None,
                    },
                );
            }
            id.clone()
        }
    }
}

pub(crate) fn attach(
    session: &mut Session,
    side: &mut SessionSide,
    agent: &AgentRef,
    truncated_before: Option<u64>,
    em: &mut Emitter,
) {
    let id = resolve_agent(session, agent, None, em);
    if matches!(agent, AgentRef::Root) {
        session.truncated_before = truncated_before;
    }
    side.transcripts.insert(
        id,
        TranscriptState {
            truncated_before,
            ..Default::default()
        },
    );
}

/// The transcript shrank or was replaced: its open calls can no longer be paired.
pub(crate) fn truncated(
    session: &mut Session,
    side: &mut SessionSide,
    agent: &AgentRef,
    em: &mut Emitter,
) {
    let id = resolve_agent(session, agent, None, em);
    if let Some(a) = session.agents.get_mut(&id) {
        for call in a.open.keys() {
            side.call_index.remove(call);
        }
        a.open.clear();
    }
    side.transcripts
        .insert(id.clone(), TranscriptState::default());
    if matches!(agent, AgentRef::Root) {
        session.truncated_before = None;
    }
    em.emit(&session.id, None, Event::FileTruncated { agent: id });
}

pub(crate) fn apply_lines(
    session: &mut Session,
    side: &mut SessionSide,
    agent: &AgentRef,
    lines: &[ParsedLine],
    errors: &[ParseError],
    em: &mut Emitter,
) {
    let id = resolve_agent(session, agent, first_timestamp(lines), em);
    count(&mut session.parse, lines, errors);
    let mut cx = Cx {
        session,
        side,
        agent: &id,
        em,
        at: None,
    };
    // Lines and errors arrive separately; replay them in file order.
    let (mut li, mut ei) = (0, 0);
    while li < lines.len() || ei < errors.len() {
        let take_error = match (lines.get(li), errors.get(ei)) {
            (Some(l), Some(e)) => e.offset < l.offset,
            (None, Some(_)) => true,
            _ => false,
        };
        if take_error {
            let e = &errors[ei];
            cx.at = None;
            cx.emit(Event::ParseError {
                agent: id.clone(),
                offset: e.offset,
                message: e.message.clone(),
            });
            ei += 1;
        } else {
            apply_line(&mut cx, &lines[li]);
            li += 1;
        }
    }
}

/// `cwd` of the first line that carries one.
pub(crate) fn first_cwd(lines: &[ParsedLine]) -> Option<&str> {
    lines
        .iter()
        .find_map(|l| envelope_of(&l.parsed).and_then(|e| e.cwd.as_deref()))
}

pub(crate) fn first_timestamp(lines: &[ParsedLine]) -> Option<Time> {
    lines
        .iter()
        .find_map(|l| envelope_of(&l.parsed).and_then(|e| e.timestamp))
}

fn envelope_of(p: &Parsed) -> Option<&Envelope> {
    match p {
        Parsed::Assistant(l) => Some(&l.env),
        Parsed::User(l) => Some(&l.env),
        Parsed::System(l) => Some(&l.env),
        _ => None,
    }
}

/// Mirrors the counting in `parse_batch` so the session's stats reflect what the engine
/// was fed, whichever source fed it.
fn count(stats: &mut ParseStats, lines: &[ParsedLine], errors: &[ParseError]) {
    for l in lines {
        stats.lines += 1;
        stats.bytes += l.len;
        match &l.parsed {
            Parsed::Ignored(t) => *stats.ignored_types.entry(t.clone()).or_default() += 1,
            p => {
                stats.parsed += 1;
                if let Some(v) = p.version() {
                    *stats.versions.entry(v.to_owned()).or_default() += 1;
                }
            }
        }
    }
    for e in errors {
        stats.lines += 1;
        stats.errors += 1;
        if stats.first_error.is_none() {
            stats.first_error = Some(format!("offset {}: {}", e.offset, e.message));
        }
    }
}

/// Everything one line's handling needs. `at` is the current line's timestamp.
struct Cx<'a> {
    session: &'a mut Session,
    side: &'a mut SessionSide,
    agent: &'a AgentId,
    em: &'a mut Emitter,
    at: Option<Time>,
}

impl Cx<'_> {
    fn agent_mut(&mut self) -> &mut Agent {
        self.session
            .agents
            .get_mut(self.agent)
            .expect("resolve_agent created the agent")
    }

    fn transcript(&mut self) -> &mut TranscriptState {
        self.side.transcripts.entry(self.agent.clone()).or_default()
    }

    fn emit(&mut self, event: Event) {
        self.em.emit(&self.session.id, self.at, event);
    }

    /// Session and agent fields every transcript line refreshes.
    fn envelope(&mut self, env: &Envelope) {
        self.at = env.timestamp;
        let s = &mut *self.session;
        if let Some(t) = env.timestamp {
            s.started = Some(s.started.map_or(t, |x| x.min(t)));
            s.last_write = Some(s.last_write.map_or(t, |x| x.max(t)));
        }
        if s.project.as_os_str().is_empty()
            && let Some(cwd) = &env.cwd
        {
            s.project = PathBuf::from(cwd);
        }
        if env.git_branch.is_some() {
            s.branch = env.git_branch.clone();
        }
        if env.version.is_some() {
            s.version = env.version.clone();
        }
        if env.entrypoint.is_some() {
            s.entrypoint = env.entrypoint.clone();
        }
        if let Some(t) = env.timestamp {
            let a = self.agent_mut();
            a.started = Some(a.started.map_or(t, |x| x.min(t)));
            a.last_write = Some(a.last_write.map_or(t, |x| x.max(t)));
        }
    }
}

fn apply_line(cx: &mut Cx, line: &ParsedLine) {
    cx.at = None;
    match &line.parsed {
        Parsed::Assistant(l) => {
            cx.envelope(&l.env);
            assistant(cx, l);
        }
        Parsed::User(l) => {
            cx.envelope(&l.env);
            user(cx, l);
        }
        Parsed::System(l) => {
            cx.envelope(&l.env);
            system(cx, l);
        }
        Parsed::CostState(_) => {
            // step 6: reported cost and lines added/removed.
        }
        Parsed::AgentName(n) => {
            // step 9: the registry name wins when there is one.
            if !n.is_empty() {
                cx.session.name = Some(n.clone());
            }
        }
        Parsed::CustomTitle(n) => {
            if !n.is_empty() {
                cx.session.name = Some(n.clone());
            }
        }
        Parsed::AiTitle(n) => {
            if !n.is_empty() {
                cx.session.title = Some(n.clone());
            }
        }
        Parsed::LastPrompt(n) => {
            if !n.is_empty() {
                cx.session.last_prompt = Some(n.clone());
            }
        }
        Parsed::Ignored(_) => {}
    }
}

fn assistant(cx: &mut Cx, l: &AssistantLine) {
    // step 6: usage and context fill are deduplicated by `message.id` here.
    let message_id = l.message.id.as_deref();
    for block in &l.message.content {
        match block {
            Block::ToolUse { id, name, input } => open_call(cx, id, name, input, message_id),
            Block::Text { text } => {
                if text.trim().is_empty() {
                    continue;
                }
                let t = cx.transcript();
                if message_id.is_some() && t.text_message.as_deref() == message_id {
                    continue;
                }
                t.text_message = message_id.map(str::to_owned);
                let at = cx.at;
                cx.agent_mut().push_activity(Activity::Text { at });
            }
            _ => {}
        }
    }
}

fn open_call(cx: &mut Cx, id: &str, name: &str, input: &Value, group: Option<&str>) {
    cx.transcript().seen_tool_use = true;
    let call_id = ToolCallId::from(id);
    if cx.side.call_index.contains_key(&call_id) || cx.agent_mut().open.contains_key(&call_id) {
        // The same line read twice.
        return;
    }
    let (at, me) = (cx.at, cx.agent.clone());
    let summary = summarize(name, input);
    let call = ToolCall {
        id: call_id.clone(),
        agent: me.clone(),
        name: name.to_owned(),
        category: ToolCategory::of(name),
        input: Arc::new(input.clone()),
        summary: summary.clone(),
        started: at,
        finished: None,
        status: ResultStatus::Pending,
        result_bytes: 0,
        parallel_group: group.map(str::to_owned),
    };
    cx.side.call_index.insert(call_id.clone(), me.clone());
    touch_file(cx, name, input);
    let agent = cx.agent_mut();
    agent.calls += 1;
    agent.push_activity(Activity::Call {
        call: call_id.clone(),
        name: name.to_owned(),
        summary: summary.clone(),
        at,
    });
    agent.open.insert(call_id.clone(), call);
    cx.emit(Event::ToolCallStarted {
        agent: me,
        call: call_id,
        name: name.to_owned(),
        summary,
    });
}

fn touch_file(cx: &mut Cx, name: &str, input: &Value) {
    if !matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
        return;
    }
    let path = input
        .get("file_path")
        .or_else(|| input.get("notebook_path"))
        .and_then(Value::as_str);
    let Some(path) = path else { return };
    let files = &mut cx.session.files_touched;
    if let Some(i) = files.iter().position(|f| f.path == path) {
        files.remove(i);
    }
    files.push(FileTouch {
        path: path.to_owned(),
        agent: cx.agent.clone(),
        tool: name.to_owned(),
        at: cx.at,
    });
    if files.len() > FILES_TOUCHED_CAP {
        let excess = files.len() - FILES_TOUCHED_CAP;
        files.drain(..excess);
    }
}

fn user(cx: &mut Cx, l: &UserLine) {
    match &l.message.content {
        UserContent::Text(s) => {
            // Meta lines, compaction summaries, and `<command-name>` or similar wrappers
            // are not things the human typed.
            if l.is_meta || l.is_compact_summary || s.starts_with('<') || s.trim().is_empty() {
                return;
            }
            let at = cx.at;
            cx.agent_mut().push_activity(Activity::Prompt {
                at,
                len: s.chars().count(),
            });
            if cx.session.task.is_none() && *cx.agent == cx.session.root {
                cx.session.task = Some(truncate_chars(first_line(s), TASK_CHARS));
            }
        }
        UserContent::Blocks(blocks) => {
            for b in blocks {
                if let Block::ToolResult {
                    tool_use_id,
                    is_error,
                    content,
                } = b
                {
                    close_call(cx, tool_use_id, is_error.unwrap_or(false), *content);
                }
            }
        }
    }
}

fn close_call(cx: &mut Cx, id: &str, is_error: bool, result_bytes: u64) {
    let owner = if cx.agent_mut().open.contains_key(id) {
        Some(cx.agent.clone())
    } else {
        cx.side.call_index.get(id).cloned()
    };
    let call = owner
        .as_ref()
        .and_then(|o| cx.session.agents.get_mut(o))
        .and_then(|a| a.open.shift_remove(id));
    cx.side.call_index.remove(id);
    let (Some(owner), Some(mut call)) = (owner, call) else {
        orphan(cx);
        return;
    };
    call.finished = cx.at;
    call.status = if is_error {
        ResultStatus::Error
    } else {
        ResultStatus::Ok
    };
    call.result_bytes = result_bytes;
    // step 8: `toolUseResult.isAsync` links background Agent calls here.
    cx.emit(Event::ToolCallFinished {
        agent: owner,
        call: call.id,
        status: call.status,
    });
}

fn orphan(cx: &mut Cx) {
    let t = cx.transcript();
    if t.truncated_before.is_some() && !t.seen_tool_use {
        cx.session.parse.orphan_after_truncation += 1;
    } else {
        cx.session.parse.orphan_results += 1;
    }
}

fn system(cx: &mut Cx, l: &SystemLine) {
    if l.subtype.as_deref() != Some("compact_boundary") {
        return;
    }
    let m = l.compact_metadata.clone().unwrap_or_default();
    let at = cx.at;
    cx.agent_mut().push_activity(Activity::Compaction {
        at,
        pre: m.pre_tokens,
        post: m.post_tokens,
        trigger: m.trigger.clone(),
    });
    // step 6: context fill resets to `post_tokens` here.
    let agent = cx.agent.clone();
    cx.emit(Event::Compacted {
        agent,
        pre: m.pre_tokens,
        post: m.post_tokens,
        trigger: m.trigger,
    });
}

/// One line describing a tool call from its input, at most [`SUMMARY_CHARS`] chars.
pub fn summarize(name: &str, input: &Value) -> String {
    let field = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let text = match name {
        "Bash" => field("command"),
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            field("file_path").or_else(|| field("notebook_path"))
        }
        "Grep" | "Glob" => field("pattern").map(|p| match field("path") {
            Some(path) => format!("{p} in {path}"),
            None => p,
        }),
        "Agent" | "Task" => field("description"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        "AskUserQuestion" => input
            .get("questions")
            .and_then(Value::as_array)
            .and_then(|q| q.first())
            .and_then(|q| q.get("question"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        "Skill" => field("skill"),
        _ => None,
    };
    truncate_chars(first_line(text.as_deref().unwrap_or(name)), SUMMARY_CHARS)
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

/// Cuts `s` to at most `max` chars, ending in an ellipsis when it was longer. Always
/// cuts on a char boundary.
pub fn truncate_chars(s: &str, max: usize) -> String {
    let mut idx = s.char_indices().skip(max.saturating_sub(1));
    match (idx.next(), idx.next()) {
        (Some((cut, _)), Some(_)) => format!("{}…", &s[..cut]),
        _ => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::parse::parse_line;
    use crate::engine::Engine;
    use crate::input::Input;
    use crate::model::SessionId;

    fn line(json: &str) -> ParsedLine {
        ParsedLine {
            offset: 0,
            len: json.len() as u64 + 1,
            parsed: parse_line(json.as_bytes(), 0).expect("fixture line parses"),
        }
    }

    fn feed(engine: &mut Engine, agent: AgentRef, json: &str) -> Vec<Event> {
        engine
            .apply(Input::Lines {
                session: SessionId::from("s"),
                agent,
                lines: vec![line(json)],
                errors: Vec::new(),
                end_offset: 0,
                mtime: None,
            })
            .into_iter()
            .map(|e| e.event)
            .collect()
    }

    const TOOL_USE: &str = r#"{"type":"assistant","timestamp":"2026-11-02T10:00:00.000Z","cwd":"/p","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}]}}"#;

    #[test]
    fn duplicate_tool_use_line_is_ignored() {
        let mut e = Engine::new();
        let first = feed(&mut e, AgentRef::Root, TOOL_USE);
        assert!(matches!(first[0], Event::SessionDiscovered { .. }));
        assert!(matches!(first[1], Event::ToolCallStarted { .. }));
        let again = feed(&mut e, AgentRef::Root, TOOL_USE);
        assert!(again.is_empty(), "{again:?}");
        let root = e.machine().sessions["s"].root_agent();
        assert_eq!(root.open.len(), 1);
        assert_eq!(root.calls, 1);
        assert_eq!(root.activity.len(), 1);
    }

    #[test]
    fn result_in_another_transcript_closes_via_call_index() {
        let mut e = Engine::new();
        feed(&mut e, AgentRef::Root, TOOL_USE);
        let result = r#"{"type":"user","timestamp":"2026-11-02T10:00:05.000Z","agentId":"a1","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"out","is_error":true}]}}"#;
        let events = feed(&mut e, AgentRef::Sub("a1".into()), result);
        assert!(matches!(&events[0], Event::AgentSpawned { agent, .. } if agent.as_str() == "a1"));
        assert!(matches!(
            &events[1],
            Event::ToolCallFinished { agent, call, status: ResultStatus::Error }
                if agent.as_str() == "s" && call.as_str() == "toolu_1"
        ));
        let session = &e.machine().sessions["s"];
        assert!(session.root_agent().open.is_empty());
        assert!(session.agents["a1"].open.is_empty());
        assert_eq!(session.parse.orphan_results, 0);
        assert_eq!(session.parse.orphan_after_truncation, 0);
        assert_eq!(session.root_agent().children, vec![AgentId::from("a1")]);
    }

    #[test]
    fn summary_truncates_on_a_char_boundary() {
        let long = "é".repeat(130);
        let s = summarize("Bash", &serde_json::json!({ "command": long }));
        assert_eq!(s.chars().count(), SUMMARY_CHARS);
        assert!(s.ends_with('…'));
        assert!(s.starts_with(&"é".repeat(SUMMARY_CHARS - 1)));

        let exact = "字".repeat(SUMMARY_CHARS);
        assert_eq!(
            summarize("Bash", &serde_json::json!({ "command": exact })),
            exact
        );

        let multi = "cargo test --workspace\n# second line";
        assert_eq!(
            summarize("Bash", &serde_json::json!({ "command": multi })),
            "cargo test --workspace"
        );
        assert_eq!(
            summarize(
                "Grep",
                &serde_json::json!({ "pattern": "TODO", "path": "src" })
            ),
            "TODO in src"
        );
        assert_eq!(
            summarize("Glob", &serde_json::json!({ "pattern": "**/*.rs" })),
            "**/*.rs"
        );
        assert_eq!(
            summarize(
                "AskUserQuestion",
                &serde_json::json!({ "questions": [{ "question": "Which one?" }] })
            ),
            "Which one?"
        );
        assert_eq!(summarize("Mystery", &serde_json::json!({})), "Mystery");
        assert_eq!(summarize("Read", &serde_json::json!({})), "Read");
    }
}
