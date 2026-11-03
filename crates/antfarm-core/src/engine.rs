//! The synchronous reducer: applies [`Input`]s to a [`Machine`] and emits events.
//!
//! No I/O, no wall clock, no runtime. Time comes only from transcript line timestamps
//! and from `Input::Tick`, so replay and live share this code and tests drive it
//! directly. Sources do the reading; front ends read the snapshots this produces.

use std::collections::HashMap;

use crate::claude::reduce::{self, SessionSide};
use crate::event::{Event, EventEnvelope};
use crate::input::Input;
use crate::model::{Machine, Session, SessionId, Time};
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
            Input::Registry { .. } | Input::RegistryGone { .. } => {
                // step 9: registry join, liveness, status.
            }
            Input::Attached {
                session,
                agent,
                truncated_before,
                ..
            } => {
                let (s, side) = session_entry(
                    &mut self.machine,
                    &mut self.side,
                    &session,
                    None,
                    None,
                    &mut em,
                );
                reduce::attach(s, side, &agent, truncated_before, &mut em);
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
            }
            Input::SubagentMeta { .. } | Input::Workflow { .. } => {
                // step 8: sub-agent meta join and workflow runs.
            }
            Input::CostAnchor { .. } => {
                // step 6: cost basis.
            }
            Input::Tick { now } => {
                // step 6: status derivation runs here.
                self.last_tick = Some(now);
            }
        }
        self.seq = em.seq;
        if let Some(last) = em.out.last() {
            self.last_at = Some(last.at);
        }
        em.out
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
