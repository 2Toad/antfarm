//! An agent's older calls in compact form, and one view over them and the recent ones.
//!
//! Recent calls live in [`Agent::activity`] with their inputs. When a call ages out of
//! that list, or when backfill reads it from before the tail boundary, it moves here as a
//! [`CallRecord`]: no input, no result, no "why" text, only what the swimlanes and call
//! lists need plus the byte offsets of its lines, so the call screen can read the rest
//! back from the transcript on demand.
//!
//! Records sit in immutable chunks behind `Arc`s, so cloning the model for a snapshot
//! copies pointers, not calls. The store is never serialized.

use std::sync::Arc;

use serde_json::Value;

use crate::model::{Activity, Agent, BackgroundRun, ResultStatus, Time, ToolCallId};

/// Records per chunk the live path appends to before starting a new one.
pub const CHUNK: usize = 1024;

/// Where a call's lines sit in its agent's transcript.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineRefs {
    /// Offset of the assistant line holding the `tool_use` block.
    pub call: Option<u64>,
    /// Offset of the user line holding the `tool_result` block, when it is in the same
    /// transcript as the call.
    pub result: Option<u64>,
}

/// One call, without its input or result.
#[derive(Debug, Clone, PartialEq)]
pub struct CallRecord {
    pub id: ToolCallId,
    pub name: Arc<str>,
    pub summary: Box<str>,
    pub at: Option<Time>,
    pub finished: Option<Time>,
    pub status: ResultStatus,
    pub lines: LineRefs,
    pub background: Option<Box<BackgroundRun>>,
}

/// An agent's compact calls, oldest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallStore {
    chunks: Vec<Arc<Vec<CallRecord>>>,
    len: usize,
}

impl CallStore {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &CallRecord> + Clone {
        self.chunks.iter().flat_map(|c| c.iter())
    }

    /// Adds a call newer than every call already here.
    pub fn push_back(&mut self, r: CallRecord) {
        match self.chunks.last_mut() {
            Some(last) if last.len() < CHUNK => Arc::make_mut(last).push(r),
            _ => self.chunks.push(Arc::new(vec![r])),
        }
        self.len += 1;
    }

    /// Adds calls, in file order, that are all older than every call already here.
    pub fn prepend(&mut self, calls: Vec<CallRecord>) {
        if calls.is_empty() {
            return;
        }
        self.len += calls.len();
        self.chunks.insert(0, Arc::new(calls));
    }

    /// Fills in the result of a call still pending here. Searches newest first, since a
    /// late result belongs to a call near the boundary. Returns whether it was found.
    pub fn close(
        &mut self,
        id: &str,
        finished: Option<Time>,
        status: ResultStatus,
        result_offset: Option<u64>,
    ) -> bool {
        for chunk in self.chunks.iter_mut().rev() {
            let Some(i) = chunk
                .iter()
                .rposition(|r| r.id.as_str() == id && r.status == ResultStatus::Pending)
            else {
                continue;
            };
            let r = &mut Arc::make_mut(chunk)[i];
            r.finished = finished;
            r.status = status;
            r.lines.result = result_offset;
            return true;
        }
        false
    }

    /// Removes the `n` oldest calls.
    pub fn drop_front(&mut self, mut n: usize) {
        n = n.min(self.len);
        self.len -= n;
        while n > 0 {
            let first = self.chunks[0].len();
            if first <= n {
                self.chunks.remove(0);
                n -= first;
            } else {
                Arc::make_mut(&mut self.chunks[0]).drain(..n);
                n = 0;
            }
        }
    }
}

/// One call as the front ends see it, from the compact store or from recent activity.
#[derive(Debug, Clone, Copy)]
pub struct CallView<'a> {
    pub call: &'a ToolCallId,
    pub name: &'a str,
    pub summary: &'a str,
    pub at: Option<Time>,
    pub finished: Option<Time>,
    pub status: ResultStatus,
    pub background: Option<&'a BackgroundRun>,
    /// Kept only for recent calls.
    pub input: Option<&'a Arc<Value>>,
    pub why: Option<&'a str>,
    pub lines: LineRefs,
}

impl<'a> CallView<'a> {
    fn of_record(r: &'a CallRecord) -> Self {
        Self {
            call: &r.id,
            name: &r.name,
            summary: &r.summary,
            at: r.at,
            finished: r.finished,
            status: r.status,
            background: r.background.as_deref(),
            input: None,
            why: None,
            lines: r.lines,
        }
    }

    fn of_activity(x: &'a Activity) -> Option<Self> {
        match x {
            Activity::Call {
                call,
                name,
                summary,
                at,
                finished,
                status,
                input,
                why,
                background,
                lines,
            } => Some(Self {
                call,
                name,
                summary,
                at: *at,
                finished: *finished,
                status: *status,
                background: background.as_ref(),
                input: input.as_ref(),
                why: why.as_deref(),
                lines: *lines,
            }),
            _ => None,
        }
    }
}

impl Agent {
    /// Every call this agent's model holds, oldest first: the compact store, then recent
    /// activity.
    pub fn all_calls(&self) -> impl DoubleEndedIterator<Item = CallView<'_>> + Clone {
        self.older
            .iter()
            .map(CallView::of_record)
            .chain(self.activity.iter().filter_map(CallView::of_activity))
    }

    /// The call with `id`, newest first.
    pub fn find_call(&self, id: &ToolCallId) -> Option<CallView<'_>> {
        self.all_calls().rev().find(|c| c.call == id)
    }
}

/// Turns an activity entry that aged out into a compact record. Other entries are dropped.
pub(crate) fn record_of(x: Activity) -> Option<CallRecord> {
    match x {
        Activity::Call {
            call,
            name,
            summary,
            at,
            finished,
            status,
            background,
            lines,
            ..
        } => Some(CallRecord {
            id: call,
            name: name.into(),
            summary: summary.into(),
            at,
            finished,
            status,
            lines,
            background: background.map(Box::new),
        }),
        _ => None,
    }
}

/// How much of a session's history the model holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HistoryState {
    /// Every tracked transcript was read from its first line.
    #[default]
    Complete,
    /// Older history is being read in the background: `read` of `total` bytes so far.
    Loading { read: u64, total: u64 },
    /// History before the tail boundary was skipped and is not being read.
    Partial,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, status: ResultStatus) -> CallRecord {
        CallRecord {
            id: ToolCallId::from(id),
            name: "Bash".into(),
            summary: "ls".into(),
            at: None,
            finished: None,
            status,
            lines: LineRefs::default(),
            background: None,
        }
    }

    #[test]
    fn prepend_and_push_keep_file_order() {
        let mut s = CallStore::default();
        s.push_back(rec("c", ResultStatus::Ok));
        s.prepend(vec![rec("a", ResultStatus::Ok), rec("b", ResultStatus::Ok)]);
        s.push_back(rec("d", ResultStatus::Ok));
        let ids: Vec<_> = s.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c", "d"]);
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn a_clone_shares_chunks_until_one_side_changes() {
        let mut s = CallStore::default();
        s.push_back(rec("a", ResultStatus::Pending));
        let snap = s.clone();
        assert!(Arc::ptr_eq(&s.chunks[0], &snap.chunks[0]));
        assert!(s.close("a", None, ResultStatus::Ok, Some(9)));
        assert_eq!(snap.iter().next().unwrap().status, ResultStatus::Pending);
        let r = s.iter().next().unwrap();
        assert_eq!((r.status, r.lines.result), (ResultStatus::Ok, Some(9)));
        assert!(
            !s.close("a", None, ResultStatus::Ok, None),
            "already closed"
        );
    }

    #[test]
    fn drop_front_removes_the_oldest_across_chunks() {
        let mut s = CallStore::default();
        s.prepend(vec![rec("a", ResultStatus::Ok), rec("b", ResultStatus::Ok)]);
        s.push_back(rec("c", ResultStatus::Ok));
        s.prepend(vec![rec("x", ResultStatus::Ok)]);
        s.drop_front(2);
        let ids: Vec<_> = s.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["b", "c"]);
        assert_eq!(s.len(), 2);
        s.drop_front(9);
        assert!(s.is_empty());
    }

    #[test]
    fn push_back_starts_a_new_chunk_when_full() {
        let mut s = CallStore::default();
        for i in 0..CHUNK + 1 {
            s.push_back(rec(&i.to_string(), ResultStatus::Ok));
        }
        assert_eq!(s.chunks.len(), 2);
        assert_eq!(s.len(), CHUNK + 1);
    }
}
