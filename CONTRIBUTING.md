# Contributing to antfarm

Thanks for helping out. Bug reports, fixes, and support for more Claude Code versions are all welcome.

## Reporting a bug

Run `antfarm doctor` and include its output, your Claude Code version (`claude --version`), and whether the session was started from a terminal or from VS Code. `doctor` lists what antfarm found, where it found each transcript, and any lines it couldn't decode, which is usually enough to find the problem.

Check the output before you post it. It contains your project paths and session names.

## How it fits together

- `crates/antfarm-core` reads `~/.claude` and keeps a live model of sessions, agents, and tool calls.
- `crates/antfarm-view` turns that model into the view models both front ends draw.
- `crates/antfarm` is the CLI and the terminal UI (ratatui).
- `app/` is the desktop app: Tauri 2 in `app/src-tauri`, with a Svelte 5 front end in `app/src`.
- `crates/antfarm-scrub` anonymizes real transcripts for use as test fixtures.

[docs/DESIGN.md](docs/DESIGN.md) covers the design, the on-disk formats antfarm reads, and the decisions made so far. Read the data sources section before changing parsing.

## Building from source

You need a stable Rust toolchain. For the desktop app you also need Node 20 or newer and pnpm.

```sh
cargo install --path crates/antfarm   # the CLI and terminal UI

cd app
pnpm install
pnpm tauri dev      # run the app against your live sessions
pnpm tauri build    # an unsigned antfarm.app under target/release/bundle/macos
pnpm dev:mock       # the UI alone in a browser, replaying recorded screens
```

## Tests

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && pnpm check && pnpm test
```

CI runs the same checks on every pull request.

The tests run against hand-written transcripts in `fixtures/synthetic` and against real transcripts in `fixtures/real` that were anonymized with `antfarm-scrub`.

The scrubber replaces every user-visible string with deterministic text of the same length. It keeps ids, timestamps, models, token counts, and structure. Its `--verify` pass checks that the engine reaches the same state on the scrubbed copy as on the original.

The scrambling key is in the source, so a scrubbed string can't be reversed. Someone who guesses an original word can still check the guess, so only scrub transcripts whose project names you're comfortable confirming.

The app's mock fixtures in `app/fixtures` are written by a Rust test from the same scrubbed transcripts. After changing a view model, regenerate them and review the diff:

```sh
ANTFARM_WRITE_FIXTURES=1 cargo test -p antfarm-app --test fixtures
```

## Useful environment variables

| Variable | Does |
|---|---|
| `ANTFARM_CLAUDE_HOME` | read a different folder instead of `~/.claude` |
| `ANTFARM_HOSTNAME` | the machine name shown in the header |
| `ANTFARM_LOG` | log filter for the CLI, such as `antfarm_core=debug` |

## Pull requests

Keep each pull request to one change, with tests for new behavior. Write the description for a reviewer who hasn't seen the problem: what was wrong, what changed, and how you checked it.

Releases are cut by maintainers; see [docs/RELEASING.md](docs/RELEASING.md).
