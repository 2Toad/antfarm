#!/usr/bin/env bash
# Picks the next antfarm ticket to build: the open sub-issue of tracking issue #32 with the
# highest priority label (P1 first), in #32's order within a tier, that has no open blocker,
# isn't an owner task (label `question`), and isn't already in progress or in review.
# Prints the chosen issue number on the last line, after one line per skipped issue.
# Exits 1 when nothing is ready.
set -euo pipefail
REPO=2Toad/antfarm
TRACK=32

subs=$(gh api "repos/$REPO/issues/$TRACK/sub_issues" --paginate \
  --jq '.[] | select(.state == "open") | [.number, ([.labels[].name] | join(","))] | @tsv')

best=""
best_rank=9
while IFS=$'\t' read -r n labels; do
  [ -z "$n" ] && continue
  case ",$labels," in
    *,question,*) echo "skip #$n: owner task" >&2; continue ;;
    *",in progress,"*) echo "skip #$n: in progress" >&2; continue ;;
    *",in review,"*) echo "skip #$n: in review" >&2; continue ;;
  esac
  open_blockers=$(gh api "repos/$REPO/issues/$n/dependencies/blocked_by" \
    --jq '[.[] | select(.state == "open") | "#\(.number)"] | join(" ")')
  if [ -n "$open_blockers" ]; then echo "skip #$n: blocked by $open_blockers" >&2; continue; fi
  rank=$(grep -oE '(^|,)P[1-4](,|$)' <<<"$labels" | tr -dc '1-4' | head -c1)
  rank=${rank:-5}
  if [ "$rank" -lt "$best_rank" ]; then best=$n; best_rank=$rank; fi
done <<<"$subs"

if [ -z "$best" ]; then echo "nothing is ready" >&2; exit 1; fi
echo "$best"
