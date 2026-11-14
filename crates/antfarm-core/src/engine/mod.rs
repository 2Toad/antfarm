//! The synchronous reducer: applies [`Input`]s to a [`Machine`] and emits events.
//!
//! No I/O, no wall clock, no runtime. Time comes only from transcript line timestamps
//! and from `Input::Tick`, so replay and live share this code and tests drive it
//! directly. Sources do the reading; front ends read the snapshots this produces.
//!
//! Statuses are derived, never stored by a source. After every input that touches a
//! session, and for every session on each tick, the engine recomputes the session and
//! agent statuses with [`status`] using the last tick as "now", or the session's last
//! transcript write before any tick has arrived. The rules:
//!
//! ```text
//! exited    registry entry present and the pid is dead
//! waiting   registry idle or waiting with a leaf call open: question if it is
//!           AskUserQuestion, else permission; registry waiting with nothing open is
//!           permission (a race)
//! still     registry busy and no transcript write for 15 min
//! working   registry busy and a leaf call open
//! thinking  registry busy and nothing open
//! idle      otherwise
//! ```
//!
//! The leaf is the deepest agent reachable from the root through open foreground spawn
//! calls. Without a registry entry (replay), busy means a write within the last 60 s,
//! waiting only when an AskUserQuestion is open and the transcript has gone quiet, and
//! exited never.

pub mod status;

use std::collections::HashMap;

use crate::claude::files::RegistryEntry;
use crate::claude::reduce::{self, SessionSide};
use crate::claude::{subagents, workflows};
use crate::event::{Event, EventEnvelope};
use crate::input::Input;
use crate::model::{
    Machine, RegistryInfo, RegistryStatus, Session, SessionId, SessionStatus, Time,
};
use crate::snapshot::Snapshot;

/// Applies inputs to the model and hands back the events each one caused.
#[derive(Debug)]
pub struct Engine {
    machine: Machine,
    /// Sequence number of the last envelope emitted.
    seq: u64,
    last_tick: Option<Time>,
    /// `at` of the last envelope emitted; the snapshot time when no tick has arrived.
    last_at: Option<Time>,
    /// Pairing state per session that is not part of the serialized model.
    side: HashMap<SessionId, SessionSide>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            machine: Machine::default(),
            seq: 0,
            last_tick: None,
            last_at: None,
            side: HashMap::new(),
        }
    }

    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    /// The whole model now. `seq` is the last envelope seq; `at` is the last tick, else the
    /// last envelope time, else the Unix epoch.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            seq: self.seq,
            at: self.last_tick.or(self.last_at).unwrap_or(Time::UNIX_EPOCH),
            machine: self.machine.clone(),
        }
    }

    /// Applies one input and returns the envelopes it produced, in order. Envelope `at` is
    /// the line timestamp when the event came from a line, otherwise the last tick time,
    /// otherwise the Unix epoch.
    pub fn apply(&mut self, input: Input) -> Vec<EventEnvelope> {
        let mut em = Emitter {
            seq: self.seq,
            fallback: self.last_tick.unwrap_or(Time::UNIX_EPOCH),
            out: Vec::new(),
        };
        match input {
            Input::Registry { entry, alive } => {
                if entry.session_id.is_empty() {
                    return self.finish(em);
                }
                let id = SessionId::from(entry.session_id.as_str());
                let (s, _) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &id,
                    entry.cwd.as_deref(),
                    None,
                    &mut em,
                );
                apply_registry(s, &entry, alive);
                refresh(s, self.last_tick, &mut em);
            }
            Input::RegistryGone { pid } => {
                for s in self.machine.sessions.values_mut() {
                    if s.pid == Some(pid)
                        && let Some(r) = s.registry.as_mut()
                    {
                        r.alive = false;
                        refresh(s, self.last_tick, &mut em);
                    }
                }
            }
            Input::Attached {
                session,
                agent,
                path,
                truncated_before,
                backfill,
            } => {
                let (s, side) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    None,
                    &mut em,
                );
                reduce::attach(s, side, &agent, path, truncated_before, backfill, &mut em);
                refresh(s, self.last_tick, &mut em);
            }
            Input::Relocated {
                session,
                agent,
                path,
            } => {
                let (s, _) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    None,
                    &mut em,
                );
                let id = reduce::resolve_agent(s, &agent, None, &mut em);
                if let Some(a) = s.agents.get_mut(&id) {
                    a.transcript = Some(path);
                }
            }
            Input::Backfill {
                session,
                agent,
                job,
                calls,
                files,
                forks,
                read,
                total,
                done,
            } => {
                let (s, side) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    None,
                    &mut em,
                );
                let batch = reduce::BackfillBatch {
                    job,
                    calls,
                    files,
                    forks,
                    read,
                    total,
                    done,
                };
                reduce::backfill(s, side, &agent, batch, &mut em);
            }
            Input::Lines {
                session,
                agent,
                lines,
                errors,
                ..
            } => {
                // `end_offset` and `mtime` are for step 8 (sub-agent staleness).
                let project = reduce::first_cwd(&lines);
                let at = reduce::first_timestamp(&lines);
                let (s, side) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    project,
                    at,
                    &mut em,
                );
                reduce::apply_lines(s, side, &agent, &lines, &errors, &mut em);
                refresh(s, self.last_tick, &mut em);
            }
            Input::FileTruncated { session, agent } => {
                let (s, side) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    None,
                    &mut em,
                );
                reduce::truncated(s, side, &agent, &mut em);
                refresh(s, self.last_tick, &mut em);
            }
            Input::SubagentMeta {
                session,
                agent,
                meta,
                transcript_mtime,
            } => {
                let (s, side) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    transcript_mtime,
                    &mut em,
                );
                subagents::apply_meta(s, side, &agent, &meta, transcript_mtime, &mut em);
                refresh(s, self.last_tick, &mut em);
            }
            Input::Workflow { session, file } => {
                let (s, _) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    None,
                    &mut em,
                );
                workflows::apply(s, &file);
            }
            Input::CostAnchor {
                session,
                usd,
                offset,
                at,
            } => {
                let (s, side) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    at,
                    &mut em,
                );
                reduce::cost_anchor(s, side, usd, offset, at, &mut em);
            }
            Input::SessionRepo {
                session,
                repo,
                worktree,
            } => {
                let (s, _) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    None,
                    &mut em,
                );
                s.repo = Some(repo);
                s.worktree = Some(worktree);
            }
            Input::SessionSource { session, source } => {
                let (s, _) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    None,
                    &mut em,
                );
                s.source = Some(source);
            }
            Input::Tick { now } => {
                self.last_tick = Some(now);
                em.fallback = now;
                for (id, s) in self.machine.sessions.iter_mut() {
                    let side = self.side.entry(id.clone()).or_default();
                    reduce::flush_cost(s, side, Some(now), &mut em);
                    refresh(s, Some(now), &mut em);
                }
            }
        }
        self.finish(em)
    }

    fn finish(&mut self, em: Emitter) -> Vec<EventEnvelope> {
        self.seq = em.seq;
        if let Some(last) = em.out.last() {
            self.last_at = Some(last.at);
        }
        em.out
    }
}

/// When the current status began. The registry records when its own status last
/// changed, which is exact for statuses it drives and better than the moment antfarm
/// happened to notice; it is used when it is newer than the previous change.
fn since(s: &Session, now: Time) -> Time {
    let reg = s
        .registry
        .as_ref()
        .and_then(|r| r.status_updated)
        .filter(|t| *t <= now);
    match (reg, s.status_since) {
        (Some(r), Some(prev)) if r > prev => r,
        (Some(r), None) => r,
        _ => now,
    }
}

/// Copies what the live registry says onto the session. The registry's name wins over
/// names found in the transcript.
fn apply_registry(s: &mut Session, e: &RegistryEntry, alive: bool) {
    s.pid = Some(e.pid);
    if let Some(n) = e.name.as_deref().filter(|n| !n.is_empty()) {
        s.name = Some(n.to_owned());
    }
    if e.entrypoint.is_some() {
        s.entrypoint = e.entrypoint.clone();
    }
    if s.version.is_none() {
        s.version = e.version.clone();
    }
    if s.started.is_none() {
        s.started = e.started_at.and_then(Time::from_timestamp_millis);
    }
    s.registry = Some(RegistryInfo {
        status: RegistryStatus::parse(e.status.as_deref().unwrap_or("")),
        waiting_for: e.waiting_for.clone(),
        status_updated: e.status_updated_at.and_then(Time::from_timestamp_millis),
        alive,
    });
}

/// Recomputes the session's and its agents' statuses as of `tick`, or of the session's
/// last write before any tick. Emits `SessionStatusChanged` and moves `status_since`.
fn refresh(s: &mut Session, tick: Option<Time>, em: &mut Emitter) {
    let Some(now) = tick
        .or(s.last_write)
        .or(s.registry.as_ref().and_then(|r| r.status_updated))
    else {
        return;
    };
    let next = status::session_status(s.registry.as_ref(), &s.agents, &s.root, s.last_write, now);
    if next != s.status {
        em.emit(
            &s.id,
            Some(now),
            Event::SessionStatusChanged {
                from: s.status,
                to: next,
            },
        );
        s.status = next;
        s.status_since = Some(since(s, now));
        if next == SessionStatus::Exited {
            em.emit(&s.id, Some(now), Event::SessionExited);
        }
    } else if s.status_since.is_none() {
        s.status_since = Some(since(s, now));
    }
    let leaf = status::leaf_agent(&s.agents, &s.root).clone();
    for a in s.agents.values_mut() {
        a.status = status::agent_status(a, a.id == leaf, next, now);
    }
}

/// Finds a session, creating it on first sight with `project` from the first line's
/// `cwd` when there is one.
fn session_entry<'a>(
    machine: &'a mut Machine,
    side: &'a mut HashMap<SessionId, SessionSide>,
    id: &SessionId,
    project: Option<&str>,
    at: Option<Time>,
    em: &mut Emitter,
) -> (&'a mut Session, &'a mut SessionSide) {
    if !machine.sessions.contains_key(id) {
        let project = project.map(Into::into).unwrap_or_default();
        machine
            .sessions
            .insert(id.clone(), Session::new(id.clone(), project));
        em.emit(id, at, Event::SessionDiscovered { name: None });
    }
    let session = machine.sessions.get_mut(id).expect("inserted above");
    (session, side.entry(id.clone()).or_default())
}

/// Collects envelopes for one `apply`, numbering them from the engine's last seq.
#[derive(Debug)]
pub(crate) struct Emitter {
    seq: u64,
    fallback: Time,
    out: Vec<EventEnvelope>,
}

impl Emitter {
    pub(crate) fn emit(&mut self, session: &SessionId, at: Option<Time>, event: Event) {
        self.seq += 1;
        self.out.push(EventEnvelope {
            seq: self.seq,
            at: at.unwrap_or(self.fallback),
            session: Some(session.clone()),
            event,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::AgentRef;
    use chrono::TimeZone;

    #[test]
    fn seq_is_monotonic_and_snapshot_tracks_it() {
        let mut e = Engine::new();
        assert_eq!(e.snapshot().seq, 0);
        assert_eq!(e.snapshot().at, Time::UNIX_EPOCH);
        let a = e.apply(Input::Attached {
            session: "s1".into(),
            agent: AgentRef::Root,
            path: "/x".into(),
            truncated_before: None,
            backfill: None,
        });
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].seq, 1);
        assert_eq!(a[0].at, Time::UNIX_EPOCH);
        assert!(matches!(a[0].event, Event::SessionDiscovered { .. }));
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 2, 10, 0, 0).unwrap();
        assert!(e.apply(Input::Tick { now }).is_empty());
        let b = e.apply(Input::Attached {
            session: "s2".into(),
            agent: AgentRef::Root,
            path: "/y".into(),
            truncated_before: Some(10),
            backfill: None,
        });
        assert_eq!(b[0].seq, 2);
        assert_eq!(b[0].at, now);
        let snap = e.snapshot();
        assert_eq!(snap.seq, 2);
        assert_eq!(snap.at, now);
        assert_eq!(snap.machine.sessions["s2"].truncated_before, Some(10));
        assert!(snap.machine.sessions["s1"].project.as_os_str().is_empty());
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use crate::claude::parse::{ParsedLine, parse_line};
    use crate::input::AgentRef;
    use crate::model::WaitingKind;

    fn reg(status: &str, alive: bool) -> Input {
        let entry: RegistryEntry = serde_json::from_str(&format!(
            r#"{{"pid":748,"sessionId":"s","cwd":"/w/app","startedAt":1793657740567,"name":"app","entrypoint":"claude-vscode","version":"2.1.314","status":"{status}","waitingFor":"permission prompt","statusUpdatedAt":1793657800000}}"#
        ))
        .unwrap();
        Input::Registry { entry, alive }
    }

    fn line(json: &str) -> Input {
        Input::Lines {
            session: SessionId::from("s"),
            agent: AgentRef::Root,
            lines: vec![ParsedLine {
                offset: 0,
                len: 1,
                parsed: parse_line(json.as_bytes(), 0).unwrap(),
            }],
            errors: vec![],
            end_offset: 0,
            mtime: None,
        }
    }

    fn tick(e: &mut Engine, s: &str) -> Vec<Event> {
        e.apply(Input::Tick {
            now: s.parse().unwrap(),
        })
        .into_iter()
        .map(|x| x.event)
        .collect()
    }

    #[test]
    fn registry_drives_status_name_and_exit() {
        let mut e = Engine::new();
        e.apply(reg("busy", true));
        e.apply(line(
            r#"{"type":"agent-name","agentName":"from-transcript"}"#,
        ));
        e.apply(line(r#"{"type":"assistant","timestamp":"2026-11-02T10:00:00Z","message":{"id":"m","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"pnpm db:migrate"}}]}}"#));
        let s = &e.machine().sessions["s"];
        assert_eq!(s.pid, Some(748));
        assert_eq!(s.name.as_deref(), Some("app"), "the registry name wins");
        assert_eq!(s.project.to_str(), Some("/w/app"));
        tick(&mut e, "2026-11-02T10:00:05Z");
        assert_eq!(e.machine().sessions["s"].status, SessionStatus::Working);

        e.apply(reg("waiting", true));
        assert_eq!(
            e.machine().sessions["s"].status,
            SessionStatus::Waiting(WaitingKind::Permission)
        );

        let ev: Vec<Event> = e
            .apply(Input::RegistryGone { pid: 748 })
            .into_iter()
            .map(|x| x.event)
            .collect();
        assert_eq!(e.machine().sessions["s"].status, SessionStatus::Exited);
        assert!(ev.iter().any(|x| matches!(x, Event::SessionExited)));
        assert_eq!(
            ev.iter()
                .filter(|x| matches!(x, Event::SessionExited))
                .count(),
            1
        );
        assert!(
            !tick(&mut e, "2026-11-02T10:01:00Z")
                .iter()
                .any(|x| matches!(x, Event::SessionExited))
        );
    }

    #[test]
    fn dead_pid_in_registry_is_exited_without_any_lines() {
        let mut e = Engine::new();
        e.apply(reg("idle", false));
        assert_eq!(e.machine().sessions["s"].status, SessionStatus::Exited);
    }
}
