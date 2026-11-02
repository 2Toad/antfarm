# antfarm design

Status: design stage, 2026-11-02. No code yet. This document records what the tool is, what it reads, how it is built, and the decisions made so far. Implementation plans for each slice are written separately when that slice starts.

## 1. Purpose and positioning

antfarm shows what AI coding agents on a developer's machine are doing, live. It watches every running Claude Code session on the computer, whether started from the terminal CLI or the VS Code extension, follows the sub-agents each session spawns, and renders the whole thing as a navigable, zoomable picture. It is a read-only observer in v1. It never needs hooks, settings changes, or a wrapper around the agent CLI.

The name avoids any vendor brand on purpose. The first and only v1 data source is Claude Code, but the core model is generic so other agent CLIs can be added as adapters later.

### What exists already

Claude Code 2.1.314 ships a feature-gated screen called FleetView (`claude agents`, `claude --bg`, `/background`, an on-demand daemon, gated by a remote flag). From the strings in the binary it is a job manager for background sessions launched through it: start, stop, peek, reply, rename, and past-session enumeration. It is not a tree of sub-agents and tool calls across arbitrary sessions, and it only knows about sessions started its way. antfarm positions as the observability layer that works for any session however it was started, goes one level deeper (sub-agents, tool calls, cost, context pressure), and is vendor neutral. Anthropic could widen FleetView. That is an accepted risk.

### Non-goals for v1

Starting or stopping sessions, sending input to sessions, remote machines, cloud sessions, Windows testing, any persistence of its own.

## 2. Use cases

1. **Which session needs me.** A developer runs several sessions across tmux panes and VS Code windows. One is waiting on a permission prompt, one asked a question, three are working. antfarm puts the two that need a human at the top and can raise a desktop notification when a session flips into that state.
2. **What is it doing right now.** Each session shows its current tool call in one line, how long it has been on that step, and how long the session has run.
3. **Why is this taking so long.** Drill into a session, see the agent tree and a swimlane of the last fifteen minutes, find the sub-agent that has been running a shell command for six minutes.
4. **What happened while I was away.** Scrub an agent's activity log backward in time.
5. **What is this costing.** Live cost per session and per sub-agent, burn rate per minute, and a context-fill gauge that warns before compaction.
6. **Two sessions are stepping on each other.** Two sessions in the same repo edit the same file within a short window. antfarm is the only process that sees both.
7. **Ambient monitoring.** The farm view on a second monitor: motion means work, stillness means something is waiting on you.
8. **Show someone.** Replay a transcript at speed, or export a session's swimlane as SVG or HTML for a PR or bug report.

## 3. Data sources: what Claude Code writes to disk

Everything below was observed on macOS with Claude Code 2.1.301 through 2.1.314. The formats are undocumented and change between builds. All parsing must ignore unknown fields and fail soft per line.

### 3.1 Live session registry

`~/.claude/sessions/<pid>.json`, one per running process, mode 0644. Rewritten in place when status changes. Observed fields:

| field | notes |
|---|---|
| `pid`, `procStart`, `startedAt` | `procStart` is a human date string; compare it with the live process start time to defeat pid reuse |
| `sessionId` | joins to the transcript filename |
| `cwd` | project directory at launch |
| `version` | Claude Code build |
| `kind` | `interactive` observed |
| `entrypoint` | `claude-vscode` for the extension. The CLI value is not yet observed on this machine because the standalone CLI here is 2.1.87 and predates the registry |
| `name`, `nameSource`, `nameSince` | short display name, `derived` or user set |
| `status`, `statusUpdatedAt`, `updatedAt` | `busy` or `idle`, updates live |
| `messagingSocketPath` | `/tmp/cc-socks/<pid>.sock`, the peer messaging socket |
| `peerProtocol`, `peerFeatures` | protocol version 1, features such as `notify_idle` |
| `bridgeSessionId` | remote bridge id, informational |

A sibling `<pid>.<hash>.key` file, mode 0600, 108 bytes, is almost certainly the socket auth token. antfarm v1 never reads it.

Stale entries from crashed processes exist. Liveness is checked with a zero signal to the pid plus the `procStart` comparison.

### 3.2 Main transcript

`~/.claude/projects/<project-slug>/<sessionId>.jsonl`, appended live. The slug is derived from `cwd` by an undocumented substitution (`/Users/jason/2toad/projects/antfarm` becomes `-Users-jason-2toad-projects-antfarm`). antfarm does not reimplement it. It finds the transcript by searching project directories for `<sessionId>.jsonl` and caches the result.

Files get large. Live transcripts of 150 MB to 400 MB exist on this machine.

Line `type` values seen in one recent 20,000 line window, roughly by frequency: `assistant`, `api-request-blob`, `user`, `attachment`, `api-request`, `pr-link`, `bridge-session`, `last-prompt`, `custom-title`, `agent-name`, `ai-title`, `atis-latch`, `mode`, `queue-operation`, `file-history-snapshot`, `file-history-delta`, `api-request-shape`, `cost-state`, `system`, `tools-used`. Only the following matter to antfarm:

- **`assistant`.** Top-level: `uuid`, `parentUuid`, `timestamp`, `sessionId`, `cwd`, `gitBranch`, `version`, `entrypoint`, `isSidechain`, `requestedModel`, `effort`, `thinkingDurationMs`. `message.model`, `message.stop_reason`, `message.usage` with `input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, `output_tokens`, and `output_tokens_details.thinking_tokens`. `message.content` is a list of blocks; `tool_use` blocks carry `id`, `name`, `input`.
- **`user`.** Same envelope plus `permissionDecision`, `sourceToolAssistantUUID`, `toolUseResult`. `message.content` is either a string (a human prompt) or a list of `tool_result` blocks with `tool_use_id`, `content`, `is_error`.
- **`system`** with `subtype: compact_boundary`. `compactMetadata` has `trigger`, `preTokens`, `postTokens`, `cumulativeDroppedTokens`. This resets the context-fill estimate.
- **`cost-state`.** `totalCostUSD`, `totalAPIDuration`, `totalToolDuration`, `totalLinesAdded`, `totalLinesRemoved`, `totalDuration`, `modelUsage` keyed by model id. Written periodically, so it is the authoritative cost and the per-message usage fills the gaps between writes.
- **`agent-name`, `custom-title`, `ai-title`, `last-prompt`.** Display names and the last human prompt.
- **`queue-operation`.** Enqueued task notifications, useful later for background task nodes.

"What is it doing right now" is the last `tool_use` block whose `id` has no matching `tool_result`. Several can be open at once when the model issues parallel calls. A session that is `busy` with nothing open is thinking or streaming text. A session that is `idle` with a tool call open is blocked on a permission prompt or a question, which is the "waiting for you" state.

### 3.3 Sub-agent transcripts

`~/.claude/projects/<project-slug>/<sessionId>/subagents/agent-<id>.jsonl` plus `agent-<id>.meta.json`.

Meta fields observed: `agentType`, `isBuiltIn`, `description`, `toolUseId` (the parent's `Agent` tool call id, which is how the tree is joined), `spawnDepth`, `requestShape` (`background` observed), `requestNonInteractive`, `model`, `finished`.

Transcript lines carry the same envelope as the main transcript plus `agentId`, `promptId`, `slug`, and `isSidechain: true`. They are parsed by the same code path.

### 3.4 Workflow runs

`~/.claude/projects/<project-slug>/<sessionId>/workflows/wf_<id>.json`. Fields: `runId`, `taskId`, `workflowName`, `status`, `startTime`, `durationMs`, `phases`, `workflowProgress`, `agentCount`, `totalTokens`, `totalToolCalls`, `script`, `scriptPath`, `args`, `result`, `summary`, `logs`, `defaultModel`. Phases become labeled bands in the session tree.

### 3.5 Not used in v1

Hook events (`SubagentStart`, `SubagentStop`, `PreToolUse`, `PostToolUse`, `TaskCreated`, `TaskCompleted`) need `settings.json` changes and add latency per tool call. They are a candidate second adapter, not the v1 feed. The peer messaging socket is reserved for the v2 conductor work.

## 4. Core model

Generic, not Claude-shaped. The Claude Code adapter maps files onto it.

- **Machine.** The set of live and recently exited sessions, grouped by project.
- **Project.** A directory. Derived from session `cwd`, not from the slug.
- **Session.** id, name, project, branch, entrypoint, version, status, started, last activity, cost, usage totals, context-fill estimate, root agent, workflow runs, and a list of files touched with timestamps.
- **Agent.** id, parent agent, spawning tool call id, type, description, model, effort, depth, started, finished, usage, ordered activity entries, and the set of open tool calls.
- **ToolCall.** id, agent, name, input, one-line summary, started, finished, result status (`pending`, `ok`, `error`), result size.
- **Activity entry.** A tool call, a thinking gap with duration, a human prompt, a compaction, or an agent spawn.
- **WorkflowRun** and **Phase.** Name, status, agents, timing.

Statuses: `working` (busy, tool call open), `thinking` (busy, nothing open), `waiting` (idle, tool call open), `idle` (idle, nothing open), `exited` (registry gone or pid dead), `stale` (registry present, pid dead).

The core exposes two things: a snapshot of the whole model at any moment, and a typed event stream (`SessionDiscovered`, `SessionStatusChanged`, `AgentSpawned`, `AgentFinished`, `ToolCallStarted`, `ToolCallFinished`, `CostUpdated`, `Compacted`, `SessionExited`, and so on). Front ends render from snapshots and animate from events.

## 5. Architecture

Rust, one Cargo workspace, Rust 2024 edition, tokio runtime.

- **`antfarm-core`** (library, no UI). Registry discovery, process liveness, transcript location, byte-offset tailing, file watching with `notify` (FSEvents on macOS, inotify on Linux) plus a slow poll as a fallback, permissive serde parsing, the model, snapshot and event APIs, and a `Source` trait with one implementation, `ClaudeCodeSource`. Also contains the replay source that drives the model from a transcript file at a chosen speed.
- **`antfarm`** (binary). Subcommands: default launches the TUI; `ls` prints a one-shot table; `watch --json` streams NDJSON snapshots and events; `replay <file>`; `doctor` reports what was found, what failed to parse, and the Claude Code versions seen; `export` later.
- **`antfarm-app`** (later). Tauri 2 with a Svelte or Solid front end and d3 for the swimlane and the farm canvas, consuming the same core over the same event types.

Attach strategy for large transcripts: read a bounded tail (default 16 MB, back to a line boundary) and mark history as truncated before that point. Full backfill in a background task is a later addition for the timeline. Tool call pairing across the boundary is tolerated: a `tool_result` with no known `tool_use` is dropped.

Parsing policy: every line is parsed independently, unknown line types and unknown fields are ignored, a malformed line increments a counter and is skipped, and `doctor` surfaces those counters. A list of tested Claude Code versions is kept in the README.

Platform: macOS and Linux tested. Windows compiles but is untested in v1 because the paths and the socket differ.

## 6. UX

### 6.1 Scope

`antfarm` with no arguments shows every live session on the machine. If launched inside a project, that project's sessions are grouped at the top under a header and the rest follow under "Other projects". `antfarm .` or `--here` restricts to the current project. `antfarm --session <id>` opens straight into one session.

### 6.2 Zoom model

Five levels, one navigation model everywhere: Machine, Project, Session, Agent, Tool call. Enter drills in, Esc backs out, a breadcrumb shows depth.

- **Machine.** One row per session: status glyph, name, project, branch, current activity with the tool name in a pill, step elapsed, session elapsed, cost, burn-rate sparkline, context-fill gauge. Rows that need a human float to the top with a strong edge marker. A summary strip shows sessions running, waiting count, total burn rate, total spent today. An attention queue panel lists only the sessions that need a human with what they are waiting on.
- **Project.** The same rows filtered, plus files touched recently by any session here, with a conflict badge when two sessions touched the same file.
- **Session.** The agent tree with tunnel-style connectors, model and effort per node, elapsed, tokens, current tool call on a second line. Finished sub-agents collapse to one line and fade after a configurable linger. Workflow phases render as labeled bands. A detail pane on the right. A mini swimlane of the last fifteen minutes along the bottom.
- **Agent.** The scrolling activity log with thinking gaps shown as labeled spacers and errors with a red edge. A scrubber walks back in time.
- **Tool call.** Full input and output, pretty printed, secrets masked, parent chain as a breadcrumb.

### 6.3 Views

Three renderings of the same model, toggled with one key: Tree (default), Swimlanes (time on x, one lane per agent, bars per tool call colored by tool category, spawn arrows, a draggable playhead), and Farm (sessions as chambers, agents as ants carrying a tool glyph, spawns as tunnels to side chambers, waiting sessions glow and their ants stop, finished sessions dim). Farm ships in both the TUI (a compact strip and a full-screen mode in ASCII) and the desktop app (full canvas). It is meant to be legible from across a room.

### 6.4 Interaction

v1 is read-only toward the agents. Available actions: jump to the owning terminal (tmux pane via process tree walk, VS Code window focus), open the transcript, copy session id or a resume command, pin, local rename, mute. Keyboard first with a visible hint bar: Enter drill in, Esc back, t tree, s swimlane, f farm, j jump, / search, q quit. Optional macOS notification when a session enters `waiting`.

### 6.5 Privacy

Tool inputs contain commands, file contents, and sometimes secrets. The default display is a one-line summary per tool type (command for Bash, path for Read and Edit, description for Agent, query for search tools). Full input is shown only on request in the tool call level, with common secret patterns masked. Message text bodies are never shown in v1. A `--redact` mode for screenshots can come later.

## 7. Decisions log

| # | decision | chosen | why |
|---|---|---|---|
| 1 | Language and stack | Rust workspace, Ratatui TUI first, Tauri 2 app second | single static binary, one language for core and both front ends, dense terminal UI, webview for the visual app |
| 2 | Ruled out | SwiftUI, egui, iced, Go and Bubble Tea, Electron | macOS only, weak dense layouts, second language, size |
| 3 | v1 data feed | files on disk, no hooks | zero setup, no per-call latency, works for sessions started any way |
| 4 | Attach to big transcripts | bounded 16 MB tail, truncation marker | live state needs recent lines only; 400 MB files exist |
| 5 | Session to transcript mapping | search for `<sessionId>.jsonl`, cache | slug algorithm is undocumented and breaks on worktrees and symlinks |
| 6 | Fixtures | scrubber committed, scrubbed real transcripts plus synthetic edge cases | real transcripts contain private code; structure must stay real |
| 7 | Model shape | generic model, Claude Code adapter behind a `Source` trait | hooks and other CLIs become adapters |
| 8 | Non-interactive output | `ls`, `watch --json`, `doctor` in v1 | tests the core before any UI, becomes the app bridge |
| 9 | Replay | `replay <file>` in v1 | demos, bug reports, test harness |
| 10 | Tool input display | one-line summary by default, full input on demand, secrets masked | inputs contain secrets |
| 11 | Stale registry entries | liveness by signal zero plus `procStart`, exited sessions linger greyed | pid reuse |
| 12 | Busy with nothing open | shown as thinking; idle with a call open shown as waiting for you | the two signals combine cleanly |
| 13 | Drift policy | per-line fail soft, unknown ignored, `doctor` command, tested-versions list | undocumented formats |
| 14 | License | dual MIT and Apache-2.0 | Rust convention, patent grant. Repo currently has MIT only; add Apache when the workspace is created |
| 15 | Platform claim | macOS and Linux tested, Windows untested | paths and sockets differ |
| 16 | Crate names | `antfarm-core`, `antfarm`, later `antfarm-app` | both free on crates.io; npm and PyPI `antfarm` are taken, irrelevant for now |
| 17 | Default scope | machine-wide, current project pinned | the cross-session picture is the point; `antfarm .` narrows |
| 18 | Farm view | ships in TUI and app | ambient monitoring is a distinct use case |
| 19 | Conductor | v2 goal, read-only in v1 | the peer socket is undocumented; keep a seam, confirm every state change |

## 8. Roadmap

1. **Core slice.** `antfarm-core`, the scrubber, the fixture suite, `antfarm ls`, `antfarm watch --json`, `antfarm doctor`, `antfarm replay`. Done when it tracks every live session on this machine for an hour with no parse failures on the tested versions and the fixtures cover spawn, parallel tool calls, errors, compaction, and workflow phases.
2. **TUI slice.** Machine and session levels, tree view, agent log with scrubbing, attention queue, cost and burn rate, jump to tmux pane, key hints. Stretch: swimlane and farm strip.
3. **v2 features.** Cross-session file conflicts, search, export, desktop notifications, hooks adapter, full history backfill, the conductor over the peer socket.
4. **Desktop app.** Tauri 2 with the farm canvas and swimlanes as the headline, sharing the core.

## 9. Fixtures and testing

Real transcripts live under `~/.claude/projects/` and contain private source. A scrubber in the repo reads a transcript and its sub-agent directory, replaces string values in content fields with same-length placeholder text, keeps ids, timestamps, tool names, line types, structure, and usage numbers, and writes the result under `fixtures/`. Each committed fixture notes the Claude Code version it came from. Hand-written fixtures cover shapes that are rare in real data: a `tool_result` with no `tool_use`, truncated last line, malformed line in the middle, unknown line type, parallel `tool_use` blocks in one message, and a sub-agent whose meta file arrives after its transcript.

The standalone CLI must be run once from a terminal on a current build to capture the CLI `entrypoint` value and registry shape, since every session on this machine so far came from the extension.

## 10. Open questions and risks

- FleetView could grow into the same space. Mitigation: ship the TUI fast, keep the vendor-neutral model.
- The registry only exists in recent builds. Older sessions will be invisible, which is acceptable.
- Whether `permissionDecision` or another field exposes the pending permission text, so the attention queue can show what it is waiting on rather than only that it is waiting.
- Trademark search for the name has not been done.
- Cost at list price needs a price table per model, which drifts. `cost-state` lines already carry Claude Code's own computed cost, so the table is only needed for per-sub-agent splits.
