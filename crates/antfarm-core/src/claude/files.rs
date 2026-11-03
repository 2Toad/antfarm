//! The non-transcript files: the live registry, sub-agent meta, and workflow runs.

use serde::Deserialize;

/// `~/.claude/sessions/<pid>.json`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RegistryEntry {
    pub pid: u32,
    pub session_id: String,
    pub cwd: Option<String>,
    /// Epoch milliseconds of process start.
    pub started_at: Option<i64>,
    /// ctime string in UTC; fallback when `started_at` is missing.
    pub proc_start: Option<String>,
    pub version: Option<String>,
    pub kind: Option<String>,
    pub entrypoint: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
    pub waiting_for: Option<String>,
    /// Epoch milliseconds.
    pub status_updated_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub messaging_socket_path: Option<String>,
}

/// `<sessionId>/subagents/agent-<id>.meta.json`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SubagentMeta {
    pub agent_type: Option<String>,
    pub is_built_in: Option<bool>,
    pub description: Option<String>,
    pub tool_use_id: Option<String>,
    pub parent_agent_id: Option<String>,
    pub spawn_depth: Option<u8>,
    pub request_shape: Option<String>,
    pub model: Option<String>,
    pub finished: Option<bool>,
}

/// `<sessionId>/workflows/wf_<id>.json`. Only the fields antfarm shows.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowFile {
    pub run_id: String,
    pub task_id: Option<String>,
    pub workflow_name: Option<String>,
    pub status: Option<String>,
    /// Epoch milliseconds.
    pub start_time: Option<i64>,
    pub duration_ms: Option<u64>,
    pub phases: Vec<WorkflowPhase>,
    pub workflow_progress: Vec<WorkflowProgress>,
    pub agent_count: Option<u32>,
    pub total_tokens: Option<u64>,
    pub total_tool_calls: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct WorkflowPhase {
    pub title: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowProgress {
    #[serde(rename = "type")]
    pub kind: String,
    pub index: Option<u32>,
    pub title: Option<String>,
    pub label: Option<String>,
    pub phase_index: Option<u32>,
    pub phase_title: Option<String>,
    pub agent_id: Option<String>,
    pub state: Option<String>,
    pub started_at: Option<i64>,
    pub tokens: Option<u64>,
    pub tool_calls: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_entry_with_waiting_status() {
        let e: RegistryEntry = serde_json::from_str(r#"{"pid":748,"sessionId":"s","cwd":"/x","startedAt":1793657740567,"procStart":"Mon Nov  2 22:15:40 2026","version":"2.1.314","kind":"interactive","entrypoint":"claude-vscode","name":"gs-api","status":"waiting","waitingFor":"permission prompt","statusUpdatedAt":1793657744155,"peerFeatures":["notify_idle"]}"#).unwrap();
        assert_eq!(e.pid, 748);
        assert_eq!(e.status.as_deref(), Some("waiting"));
        assert_eq!(e.waiting_for.as_deref(), Some("permission prompt"));
        assert_eq!(e.started_at, Some(1793657740567));
    }

    #[test]
    fn subagent_meta() {
        let m: SubagentMeta = serde_json::from_str(r#"{"agentType":"general-purpose","isBuiltIn":true,"description":"d","toolUseId":"toolu_1","spawnDepth":1,"requestShape":"background","requestNonInteractive":true,"model":"sonnet","finished":true}"#).unwrap();
        assert_eq!(m.tool_use_id.as_deref(), Some("toolu_1"));
        assert_eq!(m.finished, Some(true));
        assert_eq!(m.spawn_depth, Some(1));
    }

    #[test]
    fn workflow_file() {
        let w: WorkflowFile = serde_json::from_str(r#"{"runId":"wf_1","workflowName":"review","status":"completed","startTime":1,"phases":[{"title":"Claims","detail":"read"}],"workflowProgress":[{"type":"workflow_phase","index":1,"title":"Claims"},{"type":"workflow_agent","index":1,"label":"extract","phaseIndex":1,"phaseTitle":"Claims","agentId":"a1","state":"done"}],"script":"…"}"#).unwrap();
        assert_eq!(w.phases[0].title, "Claims");
        assert_eq!(w.workflow_progress[1].agent_id.as_deref(), Some("a1"));
    }
}
