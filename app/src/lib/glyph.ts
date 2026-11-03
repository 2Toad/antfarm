import type { Category, Class } from "./types";

/** The shape that repeats each state's color, so state never depends on hue alone. */
export function glyph(c: Class): string {
  switch (c) {
    case "Waiting":
      return "◆";
    case "Working":
      return "●";
    case "Finished":
    case "Idle":
      return "✓";
    case "Still":
      return "○";
    case "Exited":
      return "–";
  }
}

export const CATEGORIES: Category[] = ["shell", "read", "edit", "search", "web", "spawn", "ask"];

export function categoryOf(tool: string): Category {
  if (["Bash", "BashOutput", "KillShell", "Monitor", "PowerShell"].includes(tool)) return "shell";
  if (["Read", "NotebookRead", "LS"].includes(tool)) return "read";
  if (["Edit", "MultiEdit", "Write", "NotebookEdit"].includes(tool)) return "edit";
  if (["Grep", "Glob", "ToolSearch"].includes(tool)) return "search";
  if (["WebFetch", "WebSearch"].includes(tool)) return "web";
  if (["Agent", "Task", "Workflow"].includes(tool)) return "spawn";
  if (tool === "AskUserQuestion") return "ask";
  return "other";
}
