//! Claude-specific line handling: turns parsed transcript lines into model changes.
//!
//! The engine owns the model and calls in here once per transcript batch. Pairing state
//! that is not part of the serialized model, such as which agent owns an open call and
//! what each transcript has seen so far, lives in [`SessionSide`].

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Duration;
use serde_json::Value;

use super::line::{
    AssistantLine, Block, CostStateLine, Envelope, RawUsage, ResultBody, SystemLine, ToolUseResult,
    UserContent, UserLine,
};
use super::parse::{ParseError, Parsed, ParsedLine};
use super::price;
use crate::engine::Emitter;
use crate::event::Event;
use crate::input::AgentRef;
use crate::model::{
    Activity, Agent, AgentId, BackgroundRun, CostBasis, FileTouch, ParseStats, ResultStatus,
    Session, Time, ToolCall, ToolCallId, ToolCategory, Usage,
};

/// Longest tool call summary, in chars.
pub const SUMMARY_CHARS: usize = 120;
/// Longest session task (first prompt), in chars.
pub const TASK_CHARS: usize = 200;
/// How much of the assistant's text before a call is kept as the call's "why".
pub const WHY_CHARS: usize = 600;
/// How many touched files a session remembers, most recent last.
pub const FILES_TOUCHED_CAP: usize = 200;
/// How many recent `message.id`s each transcript remembers for usage dedupe.
pub const MESSAGE_ID_WINDOW: usize = 64;
/// `usd_per_min` is cost added over this many trailing seconds, divided by minutes.
pub const RATE_WINDOW_SECS: i64 = 5 * 60;
/// Smallest change in total cost that emits `CostUpdated`.
pub const COST_EMIT_STEP: f64 = 0.01;

/// Per-session state the engine keeps beside the model.
#[derive(Debug, Default)]
pub(crate) struct SessionSide {
    /// Which agent holds each open call, so a result written to another transcript
    /// still closes it.
    pub call_index: HashMap<ToolCallId, AgentId>,
    /// One entry per transcript, keyed by the agent it belongs to.
    pub transcripts: HashMap<AgentId, TranscriptState>,
    pub cost: CostSide,
    /// The agent that issued each spawn call, kept after the call closes.
    pub spawn_caller: HashMap<ToolCallId, AgentId>,
    /// The agent each spawn call created.
    pub spawned: HashMap<ToolCallId, AgentId>,
}

/// Cost bookkeeping that the serialized model does not carry.
#[derive(Debug, Default)]
pub(crate) struct CostSide {
    /// Byte offset in the main transcript of the cost-state the session is anchored on.
    pub anchor_offset: Option<u64>,
    /// When that anchor was written, as well as it is known: its own timestamp, else the
    /// last line timestamp seen before it. Sub-agent usage before it is not added.
    pub anchor_at: Option<Time>,
    /// Priced usage by line time, for the trailing rate.
    pub recent: VecDeque<(Time, f64)>,
    /// `(total, estimate)` of the last `CostUpdated` emitted.
    pub last_emitted: Option<(f64, bool)>,
}

#[derive(Debug, Default)]
pub(crate) struct TranscriptState {
    pub truncated_before: Option<u64>,
    /// A `tool_use` has been read from this transcript since attach or truncation.
    pub seen_tool_use: bool,
    /// The assistant message whose text block was already recorded.
    pub text_message: Option<String>,
    /// The latest text block and its message id, offered to a tool call in the same
    /// message as its "why".
    pub last_text: Option<(String, String)>,
    /// Message ids whose usage was already counted; a message is written as one line
    /// per content block, each repeating the same usage.
    pub recent_messages: VecDeque<String>,
}

impl TranscriptState {
    /// True the first time `id` is seen in the window.
    fn first_sight(&mut self, id: &str) -> bool {
        if self.recent_messages.iter().any(|m| m == id) {
            return false;
        }
        if self.recent_messages.len() == MESSAGE_ID_WINDOW {
            self.recent_messages.pop_front();
        }
        self.recent_messages.push_back(id.to_owned());
        true
    }
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
            let root = session.root.clone();
            if !session.agents.contains_key(id) {
                super::subagents::ensure_agent(session, id, &root, at, em);
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
        offset: 0,
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
    let now = cx.session.last_write;
    flush_cost(cx.session, cx.side, now, cx.em);
}

/// Recomputes the trailing rate against `now` and emits `CostUpdated` when the total
/// moved by at least [`COST_EMIT_STEP`] or the basis changed.
pub(crate) fn flush_cost(
    session: &mut Session,
    side: &mut SessionSide,
    now: Option<Time>,
    em: &mut Emitter,
) {
    if let Some(now) = now {
        let floor = now - Duration::seconds(RATE_WINDOW_SECS);
        // Entries are not in time order: a sub-agent's history can arrive after newer
        // main-transcript lines, so filter the whole window rather than popping the front.
        side.cost.recent.retain(|(t, _)| *t > floor);
        let sum: f64 = side.cost.recent.iter().map(|(_, usd)| usd).sum();
        session.cost.usd_per_min = sum / (RATE_WINDOW_SECS as f64 / 60.0);
    }
    if session.cost.basis == CostBasis::Unknown {
        return;
    }
    let total = session.cost.total();
    let estimate = session.cost.is_estimate();
    let changed = match side.cost.last_emitted {
        None => true,
        Some((last, was)) => was != estimate || (total - last).abs() >= COST_EMIT_STEP,
    };
    if changed {
        side.cost.last_emitted = Some((total, estimate));
        em.emit(
            &session.id,
            now,
            Event::CostUpdated {
                usd: total,
                estimate,
            },
        );
    }
}

/// Anchors the session's cost on a cost-state found by a backward scan. Ignored when a
/// later cost-state line was already applied.
pub(crate) fn cost_anchor(
    session: &mut Session,
    side: &mut SessionSide,
    usd: f64,
    offset: u64,
    at: Option<Time>,
    em: &mut Emitter,
) {
    if side.cost.anchor_offset.is_some_and(|o| o >= offset) {
        return;
    }
    set_anchor(session, side, usd, offset, at);
    let now = session.last_write;
    flush_cost(session, side, now, em);
}

fn set_anchor(
    session: &mut Session,
    side: &mut SessionSide,
    usd: f64,
    offset: u64,
    at: Option<Time>,
) {
    session.cost.reported_usd = Some(usd);
    session.cost.delta_usd = 0.0;
    session.cost.basis = CostBasis::Reported;
    side.cost.anchor_offset = Some(offset);
    side.cost.anchor_at = at.or(session.last_write);
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
        stats.bytes += e.len;
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
    /// Byte offset of the current line in its transcript.
    offset: u64,
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
            // An agent marked finished before its lines were read got a placeholder time;
            // it cannot have finished before its own last line.
            if let Some(f) = a.finished
                && f < t
            {
                a.finished = Some(t);
            }
        }
    }
}

fn apply_line(cx: &mut Cx, line: &ParsedLine) {
    cx.at = None;
    cx.offset = line.offset;
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
        Parsed::CostState(c) => cost_state(cx, c),
        Parsed::AgentName(n) => {
            // The registry's name wins when there is one.
            if !n.is_empty() && cx.session.registry.is_none() {
                cx.session.name = Some(n.clone());
            }
        }
        Parsed::CustomTitle(n) => {
            if !n.is_empty() && cx.session.registry.is_none() {
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
    let message_id = l.message.id.as_deref();
    let model = l
        .message
        .model
        .as_deref()
        .or(l.env.requested_model.as_deref());
    if let Some(m) = model {
        let a = cx.agent_mut();
        if a.model.as_deref() != Some(m) {
            a.model = Some(m.to_owned());
        }
    }
    if l.env.effort.is_some() {
        cx.agent_mut().effort = l.env.effort.clone();
    }
    if let Some(raw) = &l.message.usage {
        // Lines without a message id cannot be deduplicated; count each once.
        let first = match message_id {
            Some(id) => cx.transcript().first_sight(id),
            None => true,
        };
        if first {
            record_usage(cx, model, &usage_of(raw));
        }
    }
    for block in &l.message.content {
        match block {
            Block::ToolUse { id, name, input } => open_call(cx, id, name, input, message_id),
            Block::Text { text } => {
                if text.trim().is_empty() {
                    continue;
                }
                if let Some(mid) = message_id {
                    cx.transcript().last_text =
                        Some((mid.to_owned(), truncate_chars(text.trim(), WHY_CHARS)));
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
    let input_arc = Arc::new(input.clone());
    let why = match (&cx.transcript().last_text, group) {
        (Some((mid, text)), Some(g)) if mid == g => Some(text.clone()),
        _ => None,
    };
    let call = ToolCall {
        id: call_id.clone(),
        agent: me.clone(),
        name: name.to_owned(),
        category: ToolCategory::of(name),
        input: Arc::clone(&input_arc),
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
    agent.bump_minute(at, 1, 0);
    agent.push_activity(Activity::Call {
        call: call_id.clone(),
        name: name.to_owned(),
        summary: summary.clone(),
        at,
        finished: None,
        status: ResultStatus::Pending,
        input: Some(Arc::clone(&input_arc)),
        why,
        background: None,
    });
    agent.open.insert(call_id.clone(), call);
    if super::subagents::SPAWN_TOOLS.contains(&name) {
        super::subagents::spawn_opened(cx.side, &call_id, &me);
    }
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
    let (added, removed) = line_delta(name, input);
    let files = &mut cx.session.files_touched;
    let (prev_added, prev_removed) = match files.iter().position(|f| f.path == path) {
        Some(i) => {
            let f = files.remove(i);
            (f.added, f.removed)
        }
        None => (0, 0),
    };
    files.push(FileTouch {
        path: path.to_owned(),
        agent: cx.agent.clone(),
        tool: name.to_owned(),
        at: cx.at,
        added: prev_added + added,
        removed: prev_removed + removed,
    });
    if files.len() > FILES_TOUCHED_CAP {
        let excess = files.len() - FILES_TOUCHED_CAP;
        files.drain(..excess);
    }
}

/// Lines an edit adds and removes, from its input alone. Old text counts as removed and
/// new text as added, so an Edit that changes one line in a ten-line block counts ten
/// each way; a Write counts its whole content as added.
pub fn line_delta(tool: &str, input: &Value) -> (u64, u64) {
    let lines = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map(|s| s.lines().count() as u64)
            .unwrap_or(0)
    };
    match tool {
        "Edit" => (
            lines(input.get("new_string")),
            lines(input.get("old_string")),
        ),
        "MultiEdit" => input
            .get("edits")
            .and_then(Value::as_array)
            .map(|edits| {
                edits.iter().fold((0, 0), |(a, r), e| {
                    (
                        a + lines(e.get("new_string")),
                        r + lines(e.get("old_string")),
                    )
                })
            })
            .unwrap_or((0, 0)),
        "Write" => (lines(input.get("content")), 0),
        "NotebookEdit" => (lines(input.get("new_source")), 0),
        _ => (0, 0),
    }
}

fn user(cx: &mut Cx, l: &UserLine) {
    match &l.message.content {
        UserContent::Text(s) => {
            // Meta lines, compaction summaries, and `<command-name>` or similar wrappers
            // are not things the human typed.
            if s.contains("<task-notification>") {
                let at = cx.at;
                super::subagents::task_notifications(cx.session, s, at, cx.em);
                finish_background(cx.session, s, at);
            }
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
                    close_call(
                        cx,
                        tool_use_id,
                        is_error.unwrap_or(false),
                        content,
                        l.tool_use_result.as_ref(),
                    );
                }
            }
        }
    }
}

fn close_call(
    cx: &mut Cx,
    id: &str,
    is_error: bool,
    body: &ResultBody,
    result: Option<&ToolUseResult>,
) {
    let result_bytes = body.bytes;
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
    if let Some(a) = cx.session.agents.get_mut(&owner) {
        let entry = a.activity.iter_mut().rev().find_map(|x| match x {
            Activity::Call {
                call: c,
                finished,
                status,
                background,
                ..
            } if *c == call.id => Some((finished, status, background)),
            _ => None,
        });
        if let Some((finished, status, background)) = entry {
            *finished = call.finished;
            *status = call.status;
            // A background command: the call returned, the command keeps running until
            // its task notification.
            if let (Some(task), Some(output)) = (
                result.and_then(|r| r.background_task_id.as_ref()),
                &body.output_file,
            ) {
                *background = Some(BackgroundRun {
                    task_id: task.clone(),
                    output: output.clone(),
                    finished: None,
                    status: None,
                });
            }
        }
    }
    cx.emit(Event::ToolCallFinished {
        agent: owner.clone(),
        call: call.id.clone(),
        status: call.status,
    });
    if super::subagents::SPAWN_TOOLS.contains(&call.name.as_str()) {
        let at = cx.at;
        super::subagents::spawn_closed(cx.session, cx.side, &call, &owner, result, at, cx.em);
    }
}

/// Marks background commands named by terminal task notifications as finished.
fn finish_background(session: &mut Session, text: &str, at: Option<Time>) {
    for (task, status) in super::subagents::parse_task_notifications(text) {
        for a in session.agents.values_mut() {
            for x in a.activity.iter_mut().rev() {
                if let Activity::Call {
                    background: Some(b),
                    ..
                } = x
                    && b.task_id == task
                    && b.finished.is_none()
                {
                    b.finished = at.or(Some(Time::UNIX_EPOCH));
                    b.status = Some(status.to_owned());
                }
            }
        }
    }
}

fn orphan(cx: &mut Cx) {
    let t = cx.transcript();
    if t.truncated_before.is_some() && !t.seen_tool_use {
        cx.session.parse.orphan_after_truncation += 1;
    } else {
        cx.session.parse.orphan_results += 1;
    }
}

/// Converts the API's usage object, splitting cache writes when the breakdown is there.
pub fn usage_of(raw: &RawUsage) -> Usage {
    Usage {
        input: raw.input_tokens,
        output: raw.output_tokens,
        cache_read: raw.cache_read_input_tokens,
        cache_creation: raw.cache_creation_input_tokens,
        cache_creation_1h: raw
            .cache_creation
            .map_or(0, |c| c.ephemeral_1h_input_tokens)
            .min(raw.cache_creation_input_tokens),
        thinking: raw.output_tokens_details.map_or(0, |d| d.thinking_tokens),
        messages: 1,
    }
}

/// Adds one deduplicated message's usage to the agent and session, refreshes the
/// context fill, and attributes its list price.
fn record_usage(cx: &mut Cx, model: Option<&str>, u: &Usage) {
    let is_root = *cx.agent == cx.session.root;
    let window = model.and_then(price::context_window);
    let priced = model.and_then(|m| price::price_usage(m, u));
    cx.session.usage.add(u);
    let line_at = cx.at;
    let agent = cx.agent_mut();
    agent.usage.add(u);
    agent.bump_minute(line_at, 0, u.input + u.cache_creation + u.output);
    agent.context.set(u.prompt_tokens(), window);
    if let Some(usd) = priced {
        agent.cost_usd = Some(agent.cost_usd.unwrap_or(0.0) + usd);
    }
    if is_root {
        cx.session.context = agent.context;
    }
    let Some(usd) = priced else { return };
    let (at, offset) = (cx.at, cx.offset);
    let cost = &mut cx.side.cost;
    let after_anchor = match cost.anchor_offset {
        None => true,
        Some(anchor) if is_root => offset > anchor,
        Some(_) => match (cost.anchor_at, at) {
            (Some(anchor_at), Some(t)) => t > anchor_at,
            _ => true,
        },
    };
    if after_anchor {
        if cx.session.cost.basis == CostBasis::Unknown {
            cx.session.cost.basis = CostBasis::TailEstimate;
        }
        cx.session.cost.delta_usd += usd;
    }
    if let Some(t) = at {
        cost.recent.push_back((t, usd));
    }
}

fn cost_state(cx: &mut Cx, c: &CostStateLine) {
    if *cx.agent != cx.session.root {
        return;
    }
    if let Some(usd) = c.total_cost_usd {
        let (offset, at) = (cx.offset, c.timestamp);
        set_anchor(cx.session, cx.side, usd, offset, at);
    }
    if c.total_lines_added.is_some() {
        cx.session.lines_added = c.total_lines_added;
    }
    if c.total_lines_removed.is_some() {
        cx.session.lines_removed = c.total_lines_removed;
    }
}

fn system(cx: &mut Cx, l: &SystemLine) {
    if l.subtype.as_deref() != Some("compact_boundary") {
        return;
    }
    let m = l.compact_metadata.clone().unwrap_or_default();
    let at = cx.at;
    let is_root = *cx.agent == cx.session.root;
    let agent = cx.agent_mut();
    agent.push_activity(Activity::Compaction {
        at,
        pre: m.pre_tokens,
        post: m.post_tokens,
        trigger: m.trigger.clone(),
    });
    if let Some(post) = m.post_tokens {
        let window = agent.context.window;
        agent.context.set(post, window);
        if is_root {
            cx.session.context = agent.context;
        }
    }
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
    use crate::model::{CostBasis, SessionId};

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
    fn usage_counts_once_per_message_id() {
        let mut e = Engine::new();
        let usage = r#""usage":{"input_tokens":10,"cache_creation_input_tokens":1000,"cache_read_input_tokens":20000,"output_tokens":100,"output_tokens_details":{"thinking_tokens":30},"cache_creation":{"ephemeral_5m_input_tokens":750,"ephemeral_1h_input_tokens":250}}"#;
        let mk = |uuid: &str, block: &str| {
            format!(
                r#"{{"type":"assistant","uuid":"{uuid}","timestamp":"2026-11-02T10:00:02.000Z","cwd":"/p","requestedModel":"opus","effort":"max","message":{{"id":"msg_9","model":"claude-opus-5-5","content":[{block}],{usage}}}}}"#
            )
        };
        let lines = vec![
            line(&mk(
                "a",
                r#"{"type":"thinking","thinking":"x","signature":"s"}"#,
            )),
            line(&mk("b", r#"{"type":"text","text":"hello"}"#)),
            line(&mk(
                "c",
                r#"{"type":"tool_use","id":"toolu_9","name":"Bash","input":{"command":"ls"}}"#,
            )),
        ];
        let events: Vec<Event> = e
            .apply(Input::Lines {
                session: SessionId::from("s"),
                agent: AgentRef::Root,
                lines,
                errors: Vec::new(),
                end_offset: 0,
                mtime: None,
            })
            .into_iter()
            .map(|e| e.event)
            .collect();
        let session = &e.machine().sessions["s"];
        let root = session.root_agent();
        assert_eq!(root.usage.messages, 1);
        assert_eq!(root.usage.input, 10);
        assert_eq!(root.usage.cache_creation, 1000);
        assert_eq!(root.usage.cache_creation_1h, 250);
        assert_eq!(root.usage.thinking, 30);
        assert_eq!(session.usage, root.usage);
        assert_eq!(root.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(root.effort.as_deref(), Some("max"));
        assert_eq!(root.context.tokens, 21010);
        assert_eq!(root.context.window, Some(1_000_000));
        assert_eq!(session.context, root.context);
        // 10 in at 4, 100 out at 20, 20000 read at 0.2, 750 at 5, 250 at 8, per MTok.
        let want = (40.0 + 2000.0 + 4000.0 + 3750.0 + 2000.0) / 1e6;
        assert!((root.cost_usd.unwrap() - want).abs() < 1e-12);
        assert_eq!(session.cost.basis, CostBasis::TailEstimate);
        assert!((session.cost.delta_usd - want).abs() < 1e-12);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::CostUpdated { estimate: true, .. })),
            "{events:?}"
        );

        // A second message with a new id counts again; its first line is enough.
        let again = mk("d", r#"{"type":"text","text":"more"}"#).replace("msg_9", "msg_10");
        feed(&mut e, AgentRef::Root, &again);
        assert_eq!(e.machine().sessions["s"].root_agent().usage.messages, 2);
    }

    #[test]
    fn cost_state_anchors_and_later_usage_adds_to_delta() {
        let mut e = Engine::new();
        let cs = r#"{"type":"cost-state","sessionId":"s","totalCostUSD":3.5,"totalLinesAdded":7,"totalLinesRemoved":1}"#;
        let msg = |id: &str, ts: &str| {
            format!(
                r#"{{"type":"assistant","timestamp":"{ts}","cwd":"/p","message":{{"id":"{id}","model":"claude-sonnet-5-5","content":[{{"type":"text","text":"t"}}],"usage":{{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":100000}}}}}}"#
            )
        };
        // Offsets increase in file order; the cost-state sits between two messages.
        let mut offset = 0;
        let mut at = |json: &str| {
            let mut l = line(json);
            l.offset = offset;
            offset += l.len;
            l
        };
        let lines = vec![
            at(&msg("m1", "2026-11-02T10:00:00.000Z")),
            at(cs),
            at(&msg("m2", "2026-11-02T10:00:10.000Z")),
        ];
        e.apply(Input::Lines {
            session: SessionId::from("s"),
            agent: AgentRef::Root,
            lines,
            errors: Vec::new(),
            end_offset: offset,
            mtime: None,
        });
        let s = &e.machine().sessions["s"];
        assert_eq!(s.cost.reported_usd, Some(3.5));
        assert_eq!(s.cost.basis, CostBasis::Reported);
        // Only m2 (100k output at $10/MTok) is after the anchor.
        assert!(
            (s.cost.delta_usd - 1.0).abs() < 1e-12,
            "{}",
            s.cost.delta_usd
        );
        assert_eq!((s.lines_added, s.lines_removed), (Some(7), Some(1)));
        // Both messages are within the 5 minute window: $2 over 5 minutes.
        assert!((s.cost.usd_per_min - 0.4).abs() < 1e-12);
        // The root agent's own cost covers both messages.
        assert!((s.root_agent().cost_usd.unwrap() - 2.0).abs() < 1e-12);

        // An older anchor from a backward scan does not override a later cost-state.
        e.apply(Input::CostAnchor {
            session: SessionId::from("s"),
            usd: 1.0,
            offset: 0,
            at: None,
        });
        assert_eq!(e.machine().sessions["s"].cost.reported_usd, Some(3.5));
    }

    #[test]
    fn keeps_why_text_and_tracks_background_runs() {
        let mut e = Engine::new();
        let text = r#"{"type":"assistant","timestamp":"2026-11-03T10:00:00Z","message":{"id":"m1","content":[{"type":"text","text":"Run the tests in the background so I can keep reading."}]}}"#;
        let tool = r#"{"type":"assistant","timestamp":"2026-11-03T10:00:00Z","message":{"id":"m1","content":[{"type":"tool_use","id":"tb","name":"Bash","input":{"command":"cargo test","run_in_background":true}}]}}"#;
        let result = r#"{"type":"user","timestamp":"2026-11-03T10:00:01Z","message":{"content":[{"type":"tool_result","tool_use_id":"tb","content":"Command running in background with ID: b7. Output is being written to: /tmp/x/tasks/b7.output. You will be notified when it completes."}]},"toolUseResult":{"stdout":"","backgroundTaskId":"b7"}}"#;
        feed(&mut e, AgentRef::Root, text);
        feed(&mut e, AgentRef::Root, tool);
        feed(&mut e, AgentRef::Root, result);
        let find = |e: &Engine| {
            e.machine().sessions["s"]
                .root_agent()
                .activity
                .iter()
                .find_map(|x| match x {
                    Activity::Call {
                        call,
                        why,
                        background,
                        ..
                    } if call.as_str() == "tb" => Some((why.clone(), background.clone())),
                    _ => None,
                })
        };
        let (why, bg) = find(&e).unwrap();
        assert_eq!(
            why.as_deref(),
            Some("Run the tests in the background so I can keep reading.")
        );
        let bg = bg.unwrap();
        assert_eq!(
            (bg.task_id.as_str(), bg.output.as_str()),
            ("b7", "/tmp/x/tasks/b7.output")
        );
        assert!(bg.finished.is_none());
        let note = r#"{"type":"user","timestamp":"2026-11-03T10:05:00Z","message":{"content":"<task-notification><task-id>b7</task-id><status>completed</status></task-notification>"}}"#;
        feed(&mut e, AgentRef::Root, note);
        let bg = find(&e).unwrap().1.unwrap();
        assert_eq!(bg.status.as_deref(), Some("completed"));
        assert!(bg.finished.is_some());
    }

    #[test]
    fn line_deltas_from_edit_inputs() {
        let edit = serde_json::json!({"file_path": "a", "old_string": "one\ntwo", "new_string": "one\ntwo\nthree"});
        assert_eq!(line_delta("Edit", &edit), (3, 2));
        let multi = serde_json::json!({"edits": [{"old_string": "x", "new_string": "y\nz"}, {"old_string": "p\nq", "new_string": ""}]});
        assert_eq!(line_delta("MultiEdit", &multi), (2, 3));
        assert_eq!(
            line_delta("Write", &serde_json::json!({"content": "a\nb\nc\n"})),
            (3, 0)
        );
        assert_eq!(
            line_delta("Bash", &serde_json::json!({"command": "ls"})),
            (0, 0)
        );
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
