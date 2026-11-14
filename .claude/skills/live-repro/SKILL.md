---
name: live-repro
description: Reproduce or verify antfarm behavior against a real, throwaway Claude Code session - start one on Haiku in tmux in a scratch folder, drive it with send-keys, and watch it with `antfarm watch --json` built from main and from the branch. Use when a bug depends on live data (status, transcripts moving, sub-agents, waiting states) and fixtures can't show it, or to confirm a fix end to end.
---

# Reproduce with a live session

Fixtures cover known shapes. When a bug depends on what a current Claude Code build writes while it runs, reproduce it with a session made for the purpose and compare main with the fix.

## Rules

- Only drive sessions this skill starts. Never send keys to, or act on, the user's own sessions.
- Work in a new folder in the scratchpad, never in a real project. If the case needs git (worktrees), `git init` the scratch folder.
- Use Haiku to keep it cheap. Keep prompts small and specific to the case.
- Deleting files under `~/.claude` is the user's call. Report what was left behind instead of deleting it.

## 1. Set up

```sh
S=<scratchpad>/repro-<case>; mkdir -p "$S" && cd "$S"
git init -q && git commit -q --allow-empty -m init   # only when the case needs git
tmux new-session -d -s repro -x 160 -y 45 -c "$S" 'claude --model haiku'
```

A new folder shows the folder trust prompt first. Capture the pane (`tmux capture-pane -p -t repro`), and answer it with `tmux send-keys -t repro Enter` once you see it.

## 2. Find the session

```sh
grep -l "\"cwd\":\"$S\"" ~/.claude/sessions/*.json   # the registry entry, whose sessionId is the prefix to watch
```

Build both sides: the branch in the working tree, and main in a temporary worktree (`git worktree add <scratchpad>/main-tree main`) built with `~/.cargo/bin/cargo build -p antfarm`. Run each as `antfarm watch --json --session <prefix>` in the background, writing to its own file.

## 3. Drive it

Send a prompt with `tmux send-keys -t repro '<prompt>'`, then `Enter` as a separate send-keys after a short pause; sent together, the Enter can land before the text. Capture the pane to see where it is. Approve or deny permission prompts the case needs with the keys shown on screen.

Typical cases:

- Worktree move: ask it to call EnterWorktree, then run a few tool calls.
- Waiting states: ask for a command that needs permission, or ask it to use AskUserQuestion.
- Sub-agents: ask it to spawn a background Agent that runs two short commands.

## 4. Compare

Read both watch outputs with `jq` and compare the facts that matter: status changes, call counts, open calls, agents. The transcript on disk is the reference: count `tool_use` blocks in the `.jsonl` and compare. State the result as numbers, for example "main froze at 1 call; the branch shows 5, matching the transcript".

## 5. Clean up and report

- `tmux kill-session -t repro`, stop the watch commands, and remove the main worktree with `git worktree remove`.
- List the folders the session left in `~/.claude/projects` (their names end in the scratch folder's name, and a worktree case leaves a second one), and tell the user they can delete them.
- Report the steps, the Claude Code version (`claude --version`), and the comparison.
