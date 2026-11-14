---
name: check-ui
description: Check an antfarm desktop app UI change in the browser mock with Chrome DevTools - load a recorded screen or a temporary fixture, look at it at several window widths, measure alignment with a script instead of eyeballing, take screenshots, and clean up. Use when a change touches app/src (Svelte screens, components, CSS) and needs to be seen working, or when the user reports something looks off on a screen.
---

# Check an antfarm UI change

The mock runs the Svelte front end in a plain browser and replays recorded view events from `app/fixtures`, so a screen can be checked without Tauri or live sessions. See the top of `app/src/lib/bridge.ts` for how it works.

## 1. Start the mock

```sh
cd app && zsh -lic 'pnpm dev:mock --port 5290 --strictPort'
```

Run it in the background. If the port is taken, stop whatever holds it only if it was started by this session; otherwise pick another port.

## 2. Open the screen

Load the Chrome DevTools tools with ToolSearch (`select:mcp__chrome-devtools__new_page,mcp__chrome-devtools__navigate_page,mcp__chrome-devtools__resize_page,mcp__chrome-devtools__take_screenshot,mcp__chrome-devtools__evaluate_script,mcp__chrome-devtools__list_console_messages`).

- `http://localhost:5290/` shows `app/fixtures/machine.json`.
- `?variant=X` loads `app/fixtures/<screen>-X.json`, for example `?variant=conflict` on the project screen.
- The tutorial opens at first launch. To skip it, open the page with an init script that sets it dismissed: `localStorage.setItem("antfarm.tutorial", JSON.stringify({dismissed: true}))`, then reload. To check the tutorial itself, clear that key instead.
- If the page list is empty after a browser restart, open a new page.

## 3. Build the case if no fixture shows it

When no recorded screen has the shape the change needs (many sub-agents, long names, a waiting session), copy the nearest fixture to `app/fixtures/<screen>-<name>.json`, edit it, and load it with `?variant=<name>`. It is temporary: delete it before committing unless the change needs it as a permanent fixture. Committed fixtures are regenerated from the Rust side (CONTRIBUTING.md); a hand-edited one is overwritten the next time someone does that.

## 4. Look, then measure

1. Take a screenshot at 1440 wide, then at 1000 and 960 (the stacked layout starts below 1000px; the window's minimum width is 900). Add any width the change targets.
2. For alignment, sizes, overflow, and stickiness, measure with `evaluate_script` instead of judging a screenshot: compare `getBoundingClientRect()` centers and edges, check `scrollWidth > clientWidth` for overflow, scroll a container and re-measure for sticky elements. Report numbers, for example "every connector 0px off the box center".
3. Check both themes when colors change: emulate `prefers-color-scheme: light`.
4. Read the console with `list_console_messages`. Errors and Svelte warnings count as failures.

## 5. Clean up and report

- Stop the mock server (TaskStop on the background command).
- Delete temporary fixtures and confirm with `git status --short app/fixtures`.
- Report what was checked at which widths, the measurements, and the screenshots that matter. Say plainly that the mock isn't the real app, and name anything that needs `pnpm tauri dev` against live sessions (tray, notifications, go to terminal, live data timing).
