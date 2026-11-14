---
name: new-ticket
description: Add a story, spike, or task to the antfarm backlog on GitHub (2Toad/antfarm) in the established format, with a P1-P4 label, blocked-by links, a place in the build order on tracking issue #32, and updates to any issue it blocks. Use when asked to add, file, or write up a new ticket, story, spike, or feature for antfarm.
---

# New antfarm ticket

GitHub is the single source of truth for antfarm's backlog. Tracking issue [#32](https://github.com/2Toad/antfarm/issues/32) holds the build order, as a numbered list in its body and as ordered sub-issues, plus a table that maps each open question and risk to the issue that settles it. `docs/DESIGN.md` only points to #32, so don't add a checklist there.

Every new ticket must end up:

1. written in the format below,
2. labeled with its type and a priority,
3. linked to its blockers and to the issues it blocks,
4. placed in the build order on #32, both in the body and among the sub-issues.

## 1. Understand the request before writing

- Read #32 (`gh issue view 32 -R 2Toad/antfarm`) and the issues near where the new one will go, so it doesn't duplicate or contradict them.
- Read the parts of `docs/DESIGN.md` the ticket touches, and check claims against the code. Every fact in a ticket must be verified, not assumed. If a fact can't be checked from the repo or this machine, the ticket is a spike, or it gets a spike that blocks it.
- If the scope, the priority, or a design decision isn't clear from the request, ask the user with AskUserQuestion, offering options with a recommendation, before creating anything. Creating an issue publishes it on a public repo.

## 2. Pick the type

| Type | When | Labels |
|---|---|---|
| Story | Build something whose facts are known | `story`, `enhancement` |
| Spike | A time-boxed investigation that ends in a decision; use when a story depends on something unverified | `spike` |
| Bug | Something shipped behaves wrong | `bug` |
| Owner task | No code; a decision or action only the repo owner can take | `question` |

Add exactly one priority label:

- `P1`: do first.
- `P2`: do next.
- `P3`: later; has open blockers or needs setup on the user's side.
- `P4`: last; needs an investigation or an owner decision, or changes what antfarm is.

A ticket's priority is never higher than the priority of anything that blocks it.

## 3. Write the body

Follow the house writing rules: one line per paragraph and list item (never hard-wrap), no em dashes, plain direct language, and none of the banned words from the user's global instructions. Link `docs/DESIGN.md` sections with full URLs, for example `https://github.com/2Toad/antfarm/blob/main/docs/DESIGN.md#5-architecture`, because the issue is read on GitHub. Refer to other issues as `#N`.

### Story

```markdown
## Story

As <who>, I want <what>, so that <why>.

## Background

<What happens today, with the DESIGN.md section it comes from, and why that falls short. Concrete facts only.>

## Decisions

- **<Topic>:** <the choice made, and the reason in a clause>.

## Acceptance criteria

- [ ] <Observable, testable outcome.>
- [ ] <A test that proves it, named concretely where possible.>
- [ ] <README and DESIGN.md sections that change.>

## Blocked by

<#N, with why. Or "Nothing." Add soft dependencies here too, e.g. "Results stop at the 16 MB boundary until #13 lands.">
```

Leave out "Decisions" when there is nothing to decide. When a decision can't be made yet, put it in a spike rather than leaving it open here. When a story depends on a spike, say its acceptance criteria are provisional, "To be made firm by the spike. As a starting point:".

### Spike

```markdown
## Question

<One question, or a numbered list of them.>

## Why

<What depends on the answer, with links to the blocked issues and the DESIGN.md section.>

## Constraints

<Optional: safety or privacy limits, e.g. throwaway sessions only, never log key material.>

## Time box

<Hours or days.>

## Done when

- [ ] <Each question has a written answer with the Claude Code version it was checked on.>
- [ ] <Fixtures scrubbed and committed, if data was captured.>
- [ ] <DESIGN.md section updated.>
- [ ] <The blocked issue is updated with firm acceptance criteria, or closed with the reason.>

## Blocked by

Nothing.
```

### Bug and owner task

For a bug, use: what happens, steps to reproduce, what should happen, the evidence (version, output line, file and line), "Done when" checkboxes, and "Blocked by". For an owner task, use: "Task", "Why", "Done when", "Owner", and "Blocked by".

## 4. Create and link

Write the body to a file in the scratchpad, then:

```sh
url=$(gh issue create -R 2Toad/antfarm --title "<title>" --body-file <file> --label story --label enhancement --label P2)
n=${url##*/}
id=$(gh api repos/2Toad/antfarm/issues/$n --jq .id)
```

Titles describe the outcome in plain words ("Search calls, files, and agents across every session"). Spike titles start with "Spike: ".

**Blocked-by links** use the issue's database `id`, not its number:

```sh
# The new issue is blocked by #B
gh api -X POST repos/2Toad/antfarm/issues/$n/dependencies/blocked_by -F issue_id=$(gh api repos/2Toad/antfarm/issues/B --jq .id)
# The new issue blocks #C
gh api -X POST repos/2Toad/antfarm/issues/C/dependencies/blocked_by -F issue_id=$id
```

When the new issue blocks an existing one, also edit that issue's "Blocked by" section, and its acceptance criteria if they were waiting on this answer. Read the body with `gh issue view C --json body --jq .body`, change only that section, and write it back with `gh issue edit C --body-file -`. Check that its priority is still consistent.

## 5. Place it in the build order on #32

Pick its position: after everything that blocks it, before everything it blocks, and within its priority tier.

**Sub-issues.** Attach it, then move it to its position, after the sub-issue that comes just before it:

```sh
gh api -X POST repos/2Toad/antfarm/issues/32/sub_issues -F sub_issue_id=$id
gh api -X PATCH repos/2Toad/antfarm/issues/32/sub_issues/priority -F sub_issue_id=$id -F after_id=<id of the previous issue>
```

Use `before_id` instead for the first position. Check the result with `gh api repos/2Toad/antfarm/issues/32/sub_issues --paginate --jq '[.[].number] | join(" ")'`.

**Body.** Read #32's body and edit it in place, keeping everything else unchanged:

- Insert a line in the numbered list under its tier heading, in the form `N. #<n> <short title>`, adding `, blocked by #B` when it has blockers. Renumber the lines after it.
- An owner task with no code goes under "Any time, no code" instead.
- If the ticket settles an open question or risk, add a row to the table, or change the "Handled by" cell of an existing row.

Write it back with `gh issue edit 32 -R 2Toad/antfarm --body-file -`.

## 6. Verify and report

- Re-read the new issue on GitHub and check that every `#N` reference is right and that it has no leftover placeholders.
- Check the blocked-by links with `gh api repos/2Toad/antfarm/issues/$n/dependencies/blocked_by --jq '[.[].number]'`.
- Report to the user: the issue link, its type and priority, its position in the order, its blockers, the issues it blocks, every other issue you edited, and the decisions you made in the body that they may want to overrule.

Don't edit `docs/DESIGN.md` for a new ticket. A design change belongs in the pull request that implements it.
