# antfarm

Live view of what AI coding agents on your machine are doing. antfarm watches every running Claude Code session on the computer, whether it was started from the terminal or the VS Code extension, follows the sub-agents each one spawns, and shows the whole thing as a navigable tree, a timeline, and an ambient farm view. It reads the files the agent CLI already writes, so it needs no hooks, settings changes, or wrappers.

Status: design stage. Nothing is built yet. The design, the on-disk formats it reads, and the decisions made so far are in [docs/DESIGN.md](docs/DESIGN.md).

Planned shape:

- `antfarm`: a terminal UI that shows every session on the machine, which ones are waiting on you, what each agent is doing right now, cost and context pressure, and a drill-down from machine to project to session to agent to tool call.
- `antfarm ls`, `antfarm watch --json`, `antfarm doctor`, `antfarm replay <transcript>`: scriptable output from the same core.
- A desktop app on the same core with a swimlane timeline and a zoomable farm canvas, later.

Written in Rust. Single static binary. macOS and Linux.
