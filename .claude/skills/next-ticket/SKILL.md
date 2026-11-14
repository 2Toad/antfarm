---
name: next-ticket
description: Implement the next ready antfarm ticket from the GitHub backlog (tracking issue #32 in 2Toad/antfarm), meaning the highest-priority open issue with no open blocker. Plans it with the user, builds it on a branch, holds it to the definition of done, opens the PR, and updates the ticket's status on GitHub at each step. Use when asked to work on, pick up, or implement the next ticket, story, or spike.
---

# Next antfarm ticket

GitHub is the single source of truth for the backlog. Tracking issue [#32](https://github.com/2Toad/antfarm/issues/32) holds the build order as ordered sub-issues, and each issue has a `P1` to `P4` label. This skill takes one ticket from there to an open pull request. One ticket is one branch and one PR.

## Ticket status

Status lives on the issue, so anyone reading GitHub can see it. Create the two status labels once, if they're missing:

```sh
gh label create "in progress" -R 2Toad/antfarm --color 1d76db --description "Being built" 2>/dev/null || true
gh label create "in review" -R 2Toad/antfarm --color 5319e7 --description "PR open, waiting on review" 2>/dev/null || true
```

| Moment | Change on the issue |
|---|---|
| Plan approved, work starts | Assign to `@me`, add `in progress`, comment with the plan |
| Scope or plan changes, or a new blocker turns up | Comment with what changed and why |
| PR opened | Swap `in progress` for `in review`, comment with the PR link |
| Work stopped without a PR | Remove `in progress`, unassign, comment with where it stopped |
| PR merged | GitHub closes the issue through `Closes #N`; remove `in review` |

Comments follow the user's rules for replies others will read: a few plain sentences in the first person, with the evidence in them, and no headings or bold labels.

## 1. Pick the ticket

Start from a clean, current `main`:

```sh
git switch main && git pull --ff-only && git status --short
```

If the tree isn't clean, stop and ask the user what to do with the changes. Don't stash or discard them.

Run the picker. It prints each skipped issue with the reason, then the chosen issue number:

```sh
.claude/skills/next-ticket/next.sh
```

It skips closed issues, owner tasks (label `question`), issues already `in progress` or `in review`, and issues with an open blocker. Among the rest it takes the highest priority, in #32's order within a tier. If the user named a ticket, use that one instead, but tell them if it is blocked or out of order.

Read the ticket in full (`gh issue view N -R 2Toad/antfarm --comments`), every issue it links to, and the `docs/DESIGN.md` sections it cites.

Check the Claude Code build too. Run `antfarm doctor` and compare the `claude versions` line with the README's "tested with Claude Code X through Y". If a running build is past the range, tell the user and offer the `new-claude-version` skill before starting. Don't run it unasked; a format change can make the ticket's facts wrong, so it's their call which goes first.

## 2. Plan and start

Once a ticket is picked, start it. Don't wait for a go unless a decision blocks the work.

1. Read the code the ticket touches, and check every fact in the ticket against it. A ticket can be out of date.
2. Write a short plan: the approach, the files that change, the tests that prove each acceptance criterion, and how it will be checked by hand.
3. When a choice is genuinely the user's (a trade-off, a scope change, or a decision in the ticket that the code shows to be wrong), ask with AskUserQuestion, offering options with the recommended one first, then carry on with their answer. Make obvious choices yourself and say so in the plan.
4. Show the plan to the user, set the status, and start building.

Set the status: assign `@me`, add `in progress`, and comment the plan in a few sentences.

```sh
gh issue edit N -R 2Toad/antfarm --add-assignee @me --add-label "in progress"
gh issue comment N -R 2Toad/antfarm --body-file <file>
```

## 3. Build

- Branch from `main` with a short kebab-case name for the change, not the issue number (`history-backfill`, `redact-mode`).
- Commit in small steps. Messages are a short imperative sentence describing the change in plain words, like the existing history ("Follow a transcript that moves when its session enters a worktree"). Never add Claude or Anthropic attribution, no `Co-Authored-By` trailer.
- Follow CONTRIBUTING.md. After changing a view model, regenerate the app's mock fixtures with `ANTFARM_WRITE_FIXTURES=1 cargo test -p antfarm-app --test fixtures` and review the diff.
- When the ticket needs real data committed, as most spikes do, capture it with the `capture-fixture` skill. Never commit an unscrubbed transcript.
- Install any missing tool with Homebrew or cargo. Don't skip a step or redesign around a missing tool.
- If the work turns up something the ticket didn't foresee, such as a new blocker, a scope change, or a finding that belongs in another ticket, stop and tell the user. Comment on the issue once they decide. File a follow-up with the `new-ticket` skill rather than widening this PR.

### Spikes

A spike's output is its "Done when" list: written answers, scrubbed fixtures, `docs/DESIGN.md` updates, and edits to the issues it unblocks, which get firm acceptance criteria or are closed with the reason. Fixtures and doc changes go through a PR like any other ticket. Edit the unblocked issues on GitHub only after the user has read the findings.

## 4. Definition of done

All of these must hold before the PR is opened. Report a failure as it is, with the output, and fix it. Never mark it done.

1. **Every acceptance criterion is met,** and each one can be pointed to: a test, a command and its output, or a screenshot. A criterion that turned out to be wrong is changed only with the user's agreement, and the change is noted on the issue.
2. **The gates pass,** the same ones CI runs. Run them from the repo root, using `zsh -lic` for pnpm:

   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo test --workspace --locked
   cd app && pnpm install --frozen-lockfile && pnpm check && pnpm test && pnpm build
   ```
3. **New behavior has tests.** That includes a regression test for any bug fixed along the way.
4. **It was seen working.**
   - A change under `app/src` is checked with the `check-ui` skill. Its measurements and the screenshots that matter go in the PR.
   - A change that depends on live data (status, transcripts, sub-agents, waiting states) is checked with the `live-repro` skill, comparing main with the branch against a throwaway session.
   - CLI and terminal UI changes are run.
   - Say in the PR what was checked by hand and what wasn't.
5. **Docs are current.** README, CONTRIBUTING.md, and the `docs/DESIGN.md` sections the ticket names are updated. Docs never hard-wrap and never use em dashes.
6. **The diff is reviewed.** Read the full `git diff main...HEAD` for stray debug output, leftover temporary fixtures, and unrelated changes.

## 5. Open the PR

```sh
git push -u origin <branch>
gh pr create -R 2Toad/antfarm --base main --title "<what changed, plain words>" --body-file <file>
```

The PR body keeps its own structure:

- What changed and why, in a few sentences, starting with the problem the user had.
- How each acceptance criterion is met, as a list that points to tests or checks.
- What was checked by hand, and what wasn't.
- Follow-ups filed, if any.
- `Closes #N` on its own line, so the merge closes the issue.

No attribution lines. Then update the status: swap `in progress` for `in review`, and comment the PR link with one sentence on what's in it.

```sh
gh issue edit N -R 2Toad/antfarm --remove-label "in progress" --add-label "in review"
gh issue comment N -R 2Toad/antfarm --body "<one sentence> <PR url>"
```

Report to the user: the PR link, how each criterion is met, what wasn't checked, and anything they should look at first. Then wait for review or "merged".

## 6. After "merged"

1. `git switch main && git pull --ff-only`, then delete the local branch, and the remote one if GitHub didn't.
2. Confirm the issue closed and remove `in review` from it.
3. Note which tickets the merge unblocked (they show in the picker's next run), and update the project memory.
4. Don't start the next ticket until the user asks, and don't cut a release; that's the `cut-release` skill, run only when the user asks for one.
