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
| `pid`, `procStart`, `startedAt` | `startedAt` is epoch ms and equals the process start instant; compare it with the OS process start time to defeat pid reuse. `procStart` is a ctime string in UTC, a fallback only |
| `sessionId` | joins to the transcript filename |
| `cwd` | project directory at launch |
| `version` | Claude Code build |
| `kind` | `interactive` observed |
| `entrypoint` | `claude-vscode` for the extension. The CLI value is not yet observed on this machine because the standalone CLI here is 2.1.87 and predates the registry |
| `name`, `nameSource`, `nameSince` | short display name, `derived` or user set |
| `status`, `statusUpdatedAt`, `updatedAt` | `busy`, `idle`, or `waiting`, updates live |
| `waitingFor` | present with `waiting`, a coarse label such as `permission prompt`. It once said permission while the open call was `AskUserQuestion`, so the waiting kind comes from the open tool call and this is only a fallback label |
| `messagingSocketPath` | `/tmp/cc-socks/<pid>.sock`, the peer messaging socket |
| `peerProtocol`, `peerFeatures` | protocol version 1, features such as `notify_idle` |
| `bridgeSessionId` | remote bridge id, informational |

A sibling `<pid>.<hash>.key` file, mode 0600, 108 bytes, is almost certainly the socket auth token. antfarm v1 never reads it.

Stale entries from crashed processes exist. Liveness is a live pid whose OS start time is within five seconds of `startedAt`.

### 3.2 Main transcript

`~/.claude/projects/<project-slug>/<sessionId>.jsonl`, appended live. The slug is derived from `cwd` by an undocumented substitution (`/Users/jason/2toad/projects/antfarm` becomes `-Users-jason-2toad-projects-antfarm`). antfarm does not rely on it. It finds the transcript by searching project directories for `<sessionId>.jsonl` and caches the result. `projects/<slug>/sessions-index.json` maps some session ids to paths; it is partial and only a hint.

A session's `<sessionId>/` directory (sub-agents, workflows) can exist under several slugs at once, because git worktree slugs hold `<sessionId>/workflows` with no sibling transcript. Discovery searches every slug for `<sessionId>/` and merges what it finds.

Files get large. Live transcripts of 150 MB to 400 MB exist on this machine. Single lines reach 500 KB, the median is about 1 KB, and a byte back-seek usually lands mid-line. Each line is written with a trailing newline, but a read can land mid-write, so a trailing partial line is held back until it completes.

Line `type` values seen in one recent 20,000 line window, roughly by frequency: `assistant`, `api-request-blob`, `user`, `attachment`, `api-request`, `pr-link`, `bridge-session`, `last-prompt`, `custom-title`, `agent-name`, `ai-title`, `atis-latch`, `mode`, `queue-operation`, `file-history-snapshot`, `file-history-delta`, `api-request-shape`, `cost-state`, `system`, `tools-used`. Only the following matter to antfarm:

- **`assistant`.** Top-level: `uuid`, `parentUuid`, `timestamp`, `sessionId`, `cwd`, `gitBranch`, `version`, `entrypoint`, `isSidechain`, `requestedModel`, `effort`, `thinkingDurationMs`. `message.model`, `message.stop_reason`, `message.usage` with `input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, `output_tokens`, and `output_tokens_details.thinking_tokens`. `message.content` is a list of blocks; `tool_use` blocks carry `id`, `name`, `input`. An assistant message is written as one line per content block, each repeating the same `message.id` and the same `usage`, so usage must be deduplicated by `message.id` or cost and context are overcounted several times. `thinking` blocks carry a `signature`; `image` blocks carry base64 data.
- **`user`.** Same envelope plus `permissionDecision` (an object `{decision, source}` written after the fact, which never exposes the pending prompt), `sourceToolAssistantUUID`, `toolUseResult`, and sometimes `isMeta` or `isCompactSummary`. `message.content` is either a string (a human prompt) or a list of `tool_result` blocks with `tool_use_id`, `content`, `is_error`.
- **`system`** with `subtype: compact_boundary`. `compactMetadata` has `trigger`, `preTokens`, `postTokens`, `cumulativeDroppedTokens`. This resets the context-fill estimate. Other system subtypes such as `api_error` exist and are ignored.
- **`cost-state`.** `totalCostUSD`, `totalAPIDuration`, `totalToolDuration`, `totalLinesAdded`, `totalLinesRemoved`, `totalDuration`, `modelUsage` keyed by model id. Written rarely (six times in a 404 MB transcript), so a bounded tail usually contains none. antfarm scans backward from the end for the last one on attach and prices per-message usage after it.
- **`agent-name`, `custom-title`, `ai-title`, `last-prompt`.** Display names and the last human prompt.
- **`queue-operation`.** Enqueued task notifications, useful later for background task nodes.

"What is it doing right now" is the last `tool_use` block whose `id` has no matching `tool_result`. Several can be open at once when the model issues parallel calls. A session that is `busy` with nothing open is thinking or streaming text. A session that is `idle` with a tool call open is blocked on a permission prompt or a question, which is the "waiting for you" state.

### 3.3 Sub-agent transcripts

`~/.claude/projects/<project-slug>/<sessionId>/subagents/agent-<id>.jsonl` plus `agent-<id>.meta.json`.

Meta fields observed: `agentType`, `isBuiltIn`, `description`, `toolUseId` (the parent's `Agent` tool call id, which is how the tree is joined), `parentAgentId` (for nested foreground spawns), `spawnDepth`, `requestShape` (`background` or `foreground`), `requestNonInteractive`, `model`, `finished`. `finished` is absent while running, but about 15 percent of metas never receive it, so its absence is not proof of life; transcript mtime is the tiebreaker.

Most `Agent` calls are background. The parent's `tool_result` arrives within seconds with `toolUseResult: {isAsync: true, status: "async_launched", agentId, description, resolvedModel}`, which closes the parent's call. The sub-agent's completion then shows up as `finished: true` in its meta and as a `<task-notification>` user message in the parent carrying `<task-id>{agentId}</task-id>` and a `<status>`. Foreground `Agent` calls stay open in the parent until the sub-agent returns.

Sub-agent transcripts have no `cost-state` lines; their cost comes from per-message usage at list price.

Transcript lines carry the same envelope as the main transcript plus `agentId`, `promptId`, `slug`, and `isSidechain: true`. They are parsed by the same code path.

### 3.4 Workflow runs

`~/.claude/projects/<project-slug>/<sessionId>/workflows/wf_<id>.json`. Fields: `runId`, `taskId`, `workflowName`, `status`, `startTime`, `durationMs`, `phases`, `workflowProgress`, `agentCount`, `totalTokens`, `totalToolCalls`, `script`, `scriptPath`, `args`, `result`, `summary`, `logs`, `defaultModel`. Phases become labeled bands in the session tree. `status` has been seen as `completed` or `killed`. `phases` is `[{title, detail}]` with no state; the live detail is `workflowProgress[]`, whose `workflow_phase` and `workflow_agent` entries carry `agentId`, `state`, `startedAt`, `tokens`, `toolCalls`, and `phaseIndex`, which is the join from phase to sub-agent.

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

Statuses: `working` (busy, tool call open), `thinking` (busy, nothing open), `waiting` (idle, tool call open, split into `permission` and `question` by the pending tool), `idle` (idle, nothing open, shown as done), `still` (busy but no transcript write for fifteen minutes, a possible hang), and `exited` (registry gone or pid dead, lingers greyed then fades). Errors on a tool call are a shape, not a state.

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

The visual design lives in [design/mockup.html](design/mockup.html), produced with Claude Designer from a brief written off this document, then revised once against the data constraints in section 3. It has twelve screens: machine map in dark and light, project, session in dark and light, session swimlanes, agent with tool call, table view, menu bar tray, two 120 by 40 terminal mockups (table and session), and a states and type page. Open it in a browser. This section records what the mockup settled and what the data can and cannot back.

### 6.1 Scope

`antfarm` with no arguments shows every live session on the machine. If launched inside a project, that project's sessions are grouped at the top under a header and the rest follow under "Other projects". `antfarm .` or `--here` restricts to the current project. `antfarm --session <id>` opens straight into one session.

### 6.2 Metaphor and states

The metaphor is literal and consistent: the machine is the surface, a project is a gallery, a session is a chamber, an agent is an ant, a tool call is what the ant carries, spawned-by is a tunnel, spawn depth is depth. Only working things move. Waiting is loud and never moves. Finished settles. Still is silence.

Four primary states, one color each, with a shape that repeats the color so state never depends on hue alone, plus two minor ones:

| state | color (dark) | shape | meaning |
|---|---|---|---|
| waiting for you | coral `#FF3B7A` | filled diamond, doubled border with halo | idle with a permission request or an AskUserQuestion open |
| working | amber `#F5A623` | pulsing dot | busy, a tool call open or thinking |
| finished | green `#86A97F` | check | session or agent done |
| still | grey `#6E6961` | dashed ring, dashed tunnel | busy but no new call for fifteen minutes |
| exited | grey, dimmed | dash | process gone, fades after the linger period |
| error | none | hatched bar and a cross | a failed tool call, never a fifth color |

Palette tokens, dark: bg `0D0C0A`, panel `161512`, raise `1F1D19`, line `3D3830`, soil `14110D`, text `EEE9DF`, text-2 `B5AEA1`, text-3 `8E877B`. Light: bg `F4F1EA`, panel `FAF8F3`, raise `FFFFFF`, line `C4BBAA`, soil `E8E0D0`, text `1B1916`, text-2 `48433B`, text-3 `686257`; light accents are `E0155E`, `E08E0B`, `4F8A47`, `9C9589`. Type is Geist Mono for all data and names, Geist for prose only, with an explicit fallback stack (ui-monospace, SF Mono, Menlo, monospace).

### 6.3 Zoom model

Five levels, one navigation model everywhere: Machine, Project, Session, Agent, Tool call. Enter drills in, Esc backs out, the breadcrumb shows depth with unreached levels dimmed. A "needs you" band sits under the toolbar on every screen, showing the oldest waiting session with its request and a jump key, and "+N elsewhere" when there are more.

- **Machine.** Two renderings, Map and Table, toggled with one key. The map groups chambers by project gallery with tunnels drawn down to sub-agent chambers, an ant per agent carrying its tool glyph, a hover card per chamber, a minimap, and a live calls feed on the right with a calls-per-ten-minutes histogram. The table is htop mode: a meters strip (waiting, working, still, exited, calls/min, tok/min, spent today), a filter, a sort rule of needs-you first then time in state, a tree toggle, and one row per agent with session rows in bold. Columns: state glyph, pid (session rows only), agent, project, source, state, time in state, current or last call, calls, tok/min, cost, context fill, and a ten-minute activity sparkline. Cost is the session total on root rows and the agent's share below.
- **Project.** Sessions side by side as chambers with branch, working tree, source, files carried, and cost. A "files touched in the last ten minutes" panel with a conflict badge and explanation when two sessions edited the same file in the same working tree, and a "branches and worktrees" panel. Sessions in separate worktrees are marked "no overlap".
- **Session.** A waiting panel when applicable: the command, the reason (the Bash tool's description field), the files it touches (paths found in the input), when it was asked and after which call, and actions: go to terminal, copy command, snooze ten minutes, mute. The copy "antfarm only watches. You answer in the terminal." stays. Below it: the chamber (agent tree), a timeline with one lane per agent and a link to full swimlanes, and the tool call list newest first. The right rail holds status (state, last motion, active time, tokens, cost so far and per minute, context gauge), files changed with line deltas, the task (the first human prompt), and notify toggles.
- **Swimlanes.** Full-screen time view for one session: lanes nested by spawn depth, bars per call colored by category (shell, read, edit, search, web, spawn, ask or permission), spawn arrows, a draggable playhead with the agent tree and the call list updating to that moment, zoom and fit controls, and a jump-to-now button.
- **Agent.** The brief it was spawned with, then a call waterfall: number, tool, target, a mini timeline bar, result as a shape (ok, error cross, running, background dots), with runs of similar calls collapsed into one row. Per-agent cost in the header.
- **Tool call.** Header with running state, foreground or background tag, elapsed, copy command and go to terminal. The command, cwd, start time, and timeout. Tabs: Output, Input, Why this call (the assistant text preceding the call in the same message), Raw event. For a foreground command the Output tab shows "running, no output until the call returns" with the timer, and fills in on completion with ok or error. Only background commands stream a live tail. A footer strip shows the last completed call.

### 6.4 Terminal UI

The two 120 by 40 mockups are the Ratatui spec. Table screen, top to bottom: header line (brand, breadcrumb, spent today, clock), meters block in two columns, the needs-you box, the table, a detail box for the selected session (source, uptime, agents, calls, cost, the waiting request with its reason, last two calls, a single time bar), a status line (filter, sort, tree, rows, selected), and the key bar. Session screen: header, also-waiting line, the waiting box, agents and status boxes side by side, timeline, tool calls table, key bar.

Keys, identical across web and terminal: Enter drill in, Esc back, G go to terminal, N next waiting, / filter, S sort, T tree toggle, M map view, L swimlanes, C copy command, Z snooze, F1 or ? help, q quit, Cmd-K or / jump. Box-drawing characters and the 16-color palette only; coral maps to bright magenta, amber to yellow, green to green.

### 6.5 Desktop app additions

The menu bar tray: a badge with the waiting count, a popover listing waiting sessions first with their request and age, then working sessions with their current tool, then recently finished, and an open-antfarm shortcut. Notifications default to waiting-only.

### 6.6 What the mockup shows and how the data backs it

| shown | source |
|---|---|
| permission request with command | `tool_use` open while registry says idle; the Bash input's `command` |
| reason | the Bash input's `description` |
| question text | the open `AskUserQuestion` input |
| touches | paths parsed from the tool input |
| why this call | assistant text blocks before the `tool_use` in the same message |
| line deltas per edit | Edit and Write inputs plus `toolUseResult` |
| cost, spent today, per minute | `cost-state` lines, interpolated with per-message usage |
| per-agent cost share | per-message usage on the sub-agent transcript at list price |
| context fill | latest `cache_read_input_tokens` plus `input_tokens` over the model window, reset at `compact_boundary` |
| source app | process ancestry of the session pid; tmux pane via `tmux list-panes`; tab numbers are not knowable and are not shown |
| conflict | two sessions with the same working tree editing the same path inside the window |
| live output tail | background commands only, from their output file |
| exit code | `is_error` plus whatever the result text says; no child pid |
| snooze, mute, pin, notify | local antfarm state, never touches the session |

### 6.7 Privacy

Tool inputs contain commands, file contents, and sometimes secrets. The default display is a one-line summary per tool type (command for Bash, path for Read and Edit, description for Agent, query for search tools). Full input is shown only in the tool call level, with common secret patterns masked. Message text bodies are never shown except the task line (first human prompt) and the "why this call" excerpt. A `--redact` mode for screenshots can come later.

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
| 20 | Tree and farm views | merged into one Map view; Table is the second machine rendering | the mockup showed one screen carries both; a separate ambient view added nothing |
| 21 | TUI scope | the Table and Session terminal mockups are the v1 TUI spec, swimlanes as a stretch | they are the densest screens and the ones that ship first |
| 22 | State names | waiting, working, finished, still, exited; error is a shape | reconciles the mockup's "stale" with the doc's dead-pid case |
| 23 | Menu bar tray | part of the desktop app | cheap in Tauri, matches the attention-queue use case |
| 24 | Design source of truth | `docs/design/mockup.html` from Claude Designer, not hand-edited; changes go back through Designer | avoids forking the file |

## 8. Roadmap

1. **Core slice.** `antfarm-core`, the scrubber, the fixture suite, `antfarm ls`, `antfarm watch --json`, `antfarm doctor`, `antfarm replay`. Done when it tracks every live session on this machine for an hour with no parse failures on the tested versions and the fixtures cover spawn, parallel tool calls, errors, compaction, and workflow phases.
2. **TUI slice.** The two terminal mockups: table screen with meters, needs-you box, and selected-session detail; session screen with waiting panel, agents, status, timeline, and tool calls; agent and tool call levels as simpler lists. Jump to tmux pane. Stretch: swimlanes.
3. **v2 features.** Cross-session file conflicts, search, export, desktop notifications, hooks adapter, full history backfill, the conductor over the peer socket.
4. **Desktop app.** Tauri 2 with the map, swimlanes, project conflicts, and the menu bar tray, sharing the core.

## 9. Fixtures and testing

Real transcripts live under `~/.claude/projects/` and contain private source. A scrubber in the repo reads a transcript and its sub-agent directory, replaces string values in content fields with same-length placeholder text, keeps ids, timestamps, tool names, line types, structure, and usage numbers, and writes the result under `fixtures/`. Each committed fixture notes the Claude Code version it came from. Hand-written fixtures cover shapes that are rare in real data: a `tool_result` with no `tool_use`, truncated last line, malformed line in the middle, unknown line type, parallel `tool_use` blocks in one message, and a sub-agent whose meta file arrives after its transcript.

The standalone CLI must be run once from a terminal on a current build to capture the CLI `entrypoint` value and registry shape, since every session on this machine so far came from the extension.

## 10. Open questions and risks

- FleetView could grow into the same space. Mitigation: ship the TUI fast, keep the vendor-neutral model.
- The registry only exists in recent builds. Older sessions will be invisible, which is acceptable.
- Whether `permissionDecision` or another field exposes the pending permission text, so the attention queue can show what it is waiting on rather than only that it is waiting.
- Trademark search for the name has not been done.
- Cost at list price needs a price table per model, which drifts. `cost-state` lines already carry Claude Code's own computed cost, so the table is only needed for per-sub-agent splits.
