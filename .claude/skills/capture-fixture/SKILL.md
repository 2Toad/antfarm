---
name: capture-fixture
description: Turn a real Claude Code transcript into an anonymized antfarm test fixture with antfarm-scrub - find a session with the needed shape, scrub and verify it, check the output for leaked private strings, and add it to the test suite with its Claude Code version. Use when a ticket or spike needs real data committed (a new line type, a registry shape, a sub-agent pattern), or when a bug report needs a regression fixture.
---

# Capture a fixture

Real transcripts contain private source. Only scrubbed output is ever committed. CONTRIBUTING.md describes the scrubber; read its Tests section first.

## 1. Find a source

Prefer a session made for the purpose with the `live-repro` skill: it holds nothing private. Otherwise search the user's own transcripts for the shape, without printing their contents:

```sh
grep -l '"type":"queue-operation"' ~/.claude/projects/*/*.jsonl | head
grep -c '"type":"queue-operation"' <file>
```

Print only counts, ids, line types, and key names, never content fields. Before using a transcript from someone's real project, tell the user which project it comes from and get a yes; even scrubbed, an original word can be confirmed by guessing (CONTRIBUTING.md).

## 2. Scrub and verify

```sh
~/.cargo/bin/cargo run -q -p antfarm-scrub -- <transcript.jsonl> --out fixtures/real/<name> --verify \
  [--tail-bytes 4M] [--agents 6] [--include-agent <id>] [--registry ~/.claude/sessions/<pid>.json]
```

- `--verify` must pass: the engine reaches the same state on the scrubbed copy as on the original.
- Keep it small. Use the smallest tail that holds the shape; a fixture is read on every test run.
- Synthetic edge cases (malformed lines, a result with no call) go in `fixtures/synthetic`, written by hand, not scrubbed.

## 3. Check for leaks

The scrubber replaces content strings, but check anyway before anything is committed:

- Search the output for the project name, the user's name and username, hostnames, repo names, and distinctive words from the original prompt: `grep -rniE '<word1>|<word2>' fixtures/real/<name>`. Any hit is a blocker; report it and fix the scrubber, not the fixture.
- Check `FIXTURE.toml` lists the right `claude_code_versions` and `captured` date.
- Check the keys in a sample line (`head -c 2000 fixtures/real/<name>/main.jsonl | jq 'keys'`) for new top-level keys that carry content and should be in `--drop-keys`.

## 4. Use it

- Add a test that loads the fixture and asserts the behavior it was captured for.
- If the app's view models change, regenerate the mock fixtures (`ANTFARM_WRITE_FIXTURES=1 cargo test -p antfarm-app --test fixtures`) and review the diff.
- When the capture documents a new on-disk shape, update the matching section of `docs/DESIGN.md` section 3 with its fields and the version it was seen on.
- Run the gates. The fixture is committed in the same PR as the code or spike that needs it.
