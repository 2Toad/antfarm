---
name: new-claude-version
description: Check antfarm against a new Claude Code build - run antfarm doctor on live sessions, compare versions seen with the tested range, look for parse failures, new line types and new registry fields, and either extend the tested range or file tickets for the drift. Use when Claude Code updates, when doctor shows a version past the tested range, or when the user asks whether antfarm still works with the current Claude Code.
---

# Check a new Claude Code build

Claude Code's on-disk formats are undocumented and change between builds (`docs/DESIGN.md` section 3). The tested range is stated in the README ("tested with Claude Code X through Y"). Until #19 lands it lives only there; after #19 it is a constant in `antfarm-core`, and both must move together.

## 1. What's running

```sh
claude --version
cd <scratchpad> && ~/.cargo/bin/cargo run -q --manifest-path <repo>/Cargo.toml -p antfarm -- doctor
grep -n 'tested with Claude Code' README.md
```

From the doctor output, note: `claude versions` in Totals, `unreadable` and `oversized` line counts, `orphan results`, `price check` lines and `unpriced models`, and `skipped types`. If no live session runs the new build, start one with the `live-repro` skill and give it a few tool calls, a sub-agent, and a permission prompt, so each feature is exercised.

## 2. Look for drift

Compare against what antfarm knows. Print only counts, keys, and types, never content fields.

- **Line types:** every name in `skipped types` that `docs/DESIGN.md` section 3.2 doesn't list is new. Check whether it carries something antfarm shows (status, calls, cost, names).
- **Keys:** list top-level keys on `assistant` and `user` lines in a new-build transcript and diff them with section 3.2: `jq -c 'select(.type=="assistant") | keys' <file> | sort -u`.
- **Registry:** diff the keys of a new-build `~/.claude/sessions/<pid>.json` with the table in section 3.1.
- **Sub-agent meta:** diff `agent-*.meta.json` keys with section 3.3.
- **Behavior:** in the app or `antfarm watch --json`, check that a session's status follows working, waiting, and idle, that calls open and close, and that sub-agents appear and finish. Use `live-repro` to compare with what the session shows in its terminal.
- **Prices:** an unpriced model or a price check off by more than 5 percent means the table in `crates/antfarm-core/src/claude/price.rs` needs a row.

## 3. Act on it

- **No drift:** extend the tested range in the README (and the constant, after #19) to the new build, on a branch, in a small PR titled "Test with Claude Code <version>". The PR body lists what was checked.
- **Drift that breaks something:** file a bug with the `new-ticket` skill: the build, the symptom, the doctor line that shows it, and a scrubbed fixture if one can be captured (`capture-fixture`). Don't extend the tested range until it's fixed.
- **New data antfarm could use:** file a story or spike with `new-ticket`, and note the new fields in `docs/DESIGN.md` section 3 in the same PR that uses them.

Report the builds checked, what drifted, and the PRs or tickets opened.
