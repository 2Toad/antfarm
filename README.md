# antfarm

Live view of what AI coding agents on your machine are doing. antfarm watches every running Claude Code session on the computer, whether it was started from the terminal or the VS Code extension, follows the sub-agents each one spawns, and shows which sessions are waiting on you, what each agent is doing right now, and what it is costing. It reads the files Claude Code already writes under `~/.claude`, so it needs no hooks, settings changes, or wrappers, and it never writes to them.

Status: the terminal UI, swimlanes, the core, and the scriptable commands work. The desktop app is next. The design, the on-disk formats antfarm reads, and the decisions made so far are in [docs/DESIGN.md](docs/DESIGN.md), and the UI mockups are in [docs/design/mockup.html](docs/design/mockup.html).

## Install

From source, with a stable Rust toolchain:

```sh
cargo install --path crates/antfarm
```

## The terminal UI

Run `antfarm` in a terminal beside your sessions, for example in a tmux split or a VS Code terminal pane. When its output is not a terminal it prints the `ls` table instead, so pipes keep working.

It opens on a table of every live session and its working sub-agents, with sessions that need you at the top and a box naming what each one is waiting on. Enter drills in, Esc backs out:

- **Session:** what it is waiting on (the command and the reason given for it), its agents, status, cost, context, a timeline lane per agent, and its calls newest first.
- **Agent:** its brief and its calls, with repeated calls collapsed.
- **Call:** when it ran, how it ended, and its input with secrets masked.
- **Swimlanes (L):** one lane per agent over time, a bar per call colored by tool family, failed calls crossed out, and a playhead you can move back to see which agents were working and which calls had returned at that moment.

| key | does |
|---|---|
| Enter, Esc | drill in, back out |
| L | swimlanes: ←→ move the playhead, `,` `.` step between calls, `+` `-` zoom, F fit, End back to now |
| N | jump to the next session waiting for you |
| G | bring that session's tmux pane, editor, or terminal app forward |
| C | copy the command it is waiting on |
| Z | snooze a waiting session for ten minutes |
| M | mute a session: dimmed and sorted last |
| / | filter, with `state:waiting` and `project:api` prefixes |
| S, T | change the sort, show or hide sub-agents |
| ? | all keys |
| q | quit |

antfarm only watches. You still answer prompts in the session's own terminal.

## Commands

```sh
antfarm               # the terminal UI
antfarm ls            # one table of live sessions, waiting ones first
antfarm ls --all      # include sessions whose process is gone
antfarm ls --here     # only sessions in the current directory's project
antfarm watch         # a running log of calls, spawns, status changes, and cost
antfarm watch --json  # NDJSON: a snapshot every second plus every event
antfarm doctor        # what antfarm found, anything it could not read, and a price check
antfarm replay <transcript.jsonl> --speed 10   # play a recorded session back
```

`ls` shows, per session: state (working, thinking, waiting for a permission or an answer, idle, still, exited), how long it has been in that state, the call it is blocked on or busy with, live sub-agents, calls, cost, and how full the context window is. A cost marked `~` is estimated from the part of the transcript antfarm read; an unmarked cost is anchored on the total Claude Code itself recorded.

`doctor` is the first thing to run if something looks wrong. It lists every registry entry with whether its process is alive, where each transcript was found, how many lines were read and how many could not be decoded, and it reprices each model's lifetime usage against the cost Claude Code recorded, which shows whether antfarm's price table is current.

## Tested with

Claude Code 2.1.296 through 2.1.314 on macOS, sessions started from the VS Code extension. Linux should work and is not tested yet; Windows builds but is untested. The files antfarm reads are undocumented and change between Claude Code builds, so parsing ignores fields it does not know and skips a line it cannot read rather than failing. If `antfarm doctor` reports unreadable lines, please open an issue with its output.

## Privacy

antfarm only reads. By default it shows a one-line summary of each tool call, such as a command or a file path, never message text. A call's full input appears only on its own screen, with API keys, tokens, passwords, and URL credentials masked; masking is a heuristic, so check before you screenshot. antfarm does not send anything anywhere.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```

The tests run against hand-written transcripts in `fixtures/synthetic` and against real transcripts in `fixtures/real` that were anonymized with `antfarm-scrub`. The scrubber replaces every user-visible string with deterministic same-length text while keeping ids, timestamps, models, token counts, and structure, and its `--verify` pass checks that the engine reaches the same state on the scrubbed copy as on the original. The scrambling key is in the source, so a scrubbed string cannot be reversed, but someone who guesses an original word can check the guess; scrub only transcripts whose project names you are comfortable confirming.

## License

Dual licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
