#!/usr/bin/env python3
"""Flags writing-rule violations in a Markdown file: em dashes, banned words, and
hard-wrapped paragraphs.

As a PostToolUse hook it reads the tool call as JSON on stdin and checks the file that was
edited or written; a violation exits 2 so the report goes back to Claude to fix. Given file
paths as arguments, it checks those instead and exits 1 on a violation.
"""
import json
import re
import sys
from pathlib import Path

BANNED = [
    "delve", "leverage", "utilize", "synergy", "robust", "seamless", "groundbreaking",
    "navigate", "empowering", "moreover", "furthermore", "additionally", "consequently",
    "tapestry", "beacon", "it's worth noting", "in today's rapidly evolving landscape",
    "at the end of the day", "testament to", "navigating the complexities",
]
BANNED_RE = re.compile(r"\b(" + "|".join(re.escape(w) for w in BANNED) + r")\w*", re.IGNORECASE)
INLINE_CODE = re.compile(r"`[^`]*`")
URL = re.compile(r"\]\([^)]*\)|<https?://[^>]*>|https?://\S+")

# Lines that are one per line by definition, or that aren't prose.
STRUCTURAL = re.compile(r"^\s*(#{1,6}\s|[-*+]\s|\d+[.)]\s|>|\||<|---\s*$|\*\*\*\s*$|___\s*$|\[[^\]]+\]:\s|!\[|\[!\[)")


def check(path: Path) -> list[str]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError):
        return []
    problems = []
    fence = None
    front = bool(lines) and lines[0].strip() == "---"
    prev_prose = False
    for i, line in enumerate(lines, 1):
        stripped = line.strip()
        if front:
            if i > 1 and stripped == "---":
                front = False
            continue
        m = re.match(r"^\s*(```+|~~~+)", line)
        if m:
            marker = m.group(1)[:3]
            fence = None if fence == marker else (fence or marker)
            prev_prose = False
            continue
        if fence or line.startswith("    ") or line.startswith("\t"):
            prev_prose = False
            continue

        if "—" in line:
            problems.append(f"{path}:{i}: em dash")
        text = URL.sub("", INLINE_CODE.sub("", line))
        for w in sorted({m.group(1).lower() for m in BANNED_RE.finditer(text)}):
            problems.append(f"{path}:{i}: banned word \"{w}\"")

        is_prose = bool(stripped) and not STRUCTURAL.match(line)
        # A prose line right after another prose line is a paragraph broken mid-sentence,
        # unless the previous line ends in a deliberate hard break.
        if is_prose and prev_prose and not lines[i - 2].endswith(("  ", "\\")):
            problems.append(f"{path}:{i}: hard-wrapped paragraph (join it with the line above)")
        # A list item counts as prose for the line after it, so a list item wrapped onto a
        # second unindented line is caught too.
        prev_prose = is_prose or (bool(stripped) and re.match(r"^\s*([-*+]|\d+[.)])\s", line) is not None)
    return problems


def main() -> int:
    if len(sys.argv) > 1:
        problems = [p for a in sys.argv[1:] for p in check(Path(a))]
        print("\n".join(problems))
        return 1 if problems else 0
    try:
        call = json.load(sys.stdin)
    except json.JSONDecodeError:
        return 0
    path = (call.get("tool_input") or {}).get("file_path", "")
    if not path.endswith((".md", ".markdown")):
        return 0
    problems = check(Path(path))
    if not problems:
        return 0
    print("Writing rules broken in the file just edited. Fix these before going on:", file=sys.stderr)
    print("\n".join(problems[:40]), file=sys.stderr)
    if len(problems) > 40:
        print(f"... and {len(problems) - 40} more", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
