//! Turns a `wf_*.json` file into a [`WorkflowRun`] on its session.

use super::files::WorkflowFile;
use super::subagents::from_ms;
use crate::model::{AgentId, Phase, Session, WorkflowAgent, WorkflowRun};

/// Inserts or replaces the run with the file's `runId`. Phase indexes are 1-based, as
/// Claude Code numbers them in `workflowProgress`.
pub(crate) fn apply(session: &mut Session, file: &WorkflowFile) {
    if file.run_id.is_empty() {
        return;
    }
    let mut phases: Vec<Phase> = file
        .phases
        .iter()
        .enumerate()
        .map(|(i, p)| Phase {
            index: i as u32 + 1,
            title: p.title.clone(),
            detail: p.detail.clone(),
        })
        .collect();
    let mut agents = Vec::new();
    for p in &file.workflow_progress {
        match p.kind.as_str() {
            "workflow_phase" => {
                if let (Some(index), Some(title)) = (p.index, &p.title)
                    && !phases.iter().any(|x| x.index == index)
                {
                    phases.push(Phase {
                        index,
                        title: title.clone(),
                        detail: None,
                    });
                }
            }
            "workflow_agent" => {
                if let Some(id) = p.agent_id.as_deref().filter(|s| !s.is_empty()) {
                    agents.push(WorkflowAgent {
                        agent: AgentId::from(id),
                        label: p.label.clone(),
                        phase_index: p.phase_index,
                        state: p.state.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    phases.sort_by_key(|p| p.index);
    let run = WorkflowRun {
        run_id: file.run_id.clone(),
        name: file.workflow_name.clone(),
        status: file.status.clone(),
        started: file.start_time.and_then(from_ms),
        duration_ms: file.duration_ms,
        phases,
        agents,
    };
    match session
        .workflows
        .iter_mut()
        .find(|w| w.run_id == run.run_id)
    {
        Some(w) => *w = run,
        None => {
            session.workflows.push(run);
            session
                .workflows
                .sort_by(|a, b| a.started.cmp(&b.started).then(a.run_id.cmp(&b.run_id)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SessionId;

    #[test]
    fn builds_phases_and_agents_and_replaces_by_run_id() {
        let mut s = Session::new(SessionId::from("s"), "/p".into());
        let f: WorkflowFile = serde_json::from_str(r#"{"runId":"wf_1","workflowName":"review","status":"running","startTime":1793657740567,"phases":[{"title":"Claims","detail":"read"},{"title":"Verify"}],"workflowProgress":[{"type":"workflow_phase","index":1,"title":"Claims"},{"type":"workflow_agent","index":1,"label":"extract","phaseIndex":1,"agentId":"a1","state":"done"},{"type":"workflow_agent","index":2,"label":"check","phaseIndex":2,"agentId":"a2","state":"running"}]}"#).unwrap();
        apply(&mut s, &f);
        let w = &s.workflows[0];
        assert_eq!(
            w.phases
                .iter()
                .map(|p| (p.index, p.title.as_str()))
                .collect::<Vec<_>>(),
            [(1, "Claims"), (2, "Verify")]
        );
        assert_eq!(w.agents.len(), 2);
        assert_eq!(w.agents[1].phase_index, Some(2));
        assert!(w.started.is_some());
        let mut done = f.clone();
        done.status = Some("completed".into());
        apply(&mut s, &done);
        assert_eq!(s.workflows.len(), 1);
        assert_eq!(s.workflows[0].status.as_deref(), Some("completed"));
    }
}
