// The only place the UI talks to the Rust side. In mock mode (`pnpm dev:mock`) it
// replays recorded view events from app/fixtures instead, so screens can be checked in a
// plain browser. While the tutorial runs, the backend sends views of its sample sessions
// instead of the real ones; in mock mode they come from app/fixtures/demo, one per screen.

import type { Route, ViewEvent } from "./types";

declare const __MOCK__: boolean;
const MOCK = typeof __MOCK__ !== "undefined" && __MOCK__;

type Listener = (e: ViewEvent) => void;
const listeners: Listener[] = [];
let mockRoute: Route = { screen: "machine" };

const fixtures: Record<string, ViewEvent> = MOCK
  ? Object.fromEntries(
      Object.entries(import.meta.glob<ViewEvent>("../../fixtures/*.json", { eager: true, import: "default" })).map(([path, v]) => [
        path.split("/").pop()!.replace(/\.json$/, ""),
        v,
      ]),
    )
  : {};

/** The fixture name for a route: `machine`, `session`, `lanes`, and so on. */
function mockName(route: Route): string {
  const params = new URLSearchParams(location.search);
  const variant = params.get("variant");
  return variant ? `${route.screen}-${variant}` : route.screen;
}

function emitMock() {
  const e = fixtures[mockName(mockRoute)] ?? fixtures[mockRoute.screen];
  if (e) for (const l of listeners) l(e);
}

/** The tutorial is running: only views of its sample sessions are shown. */
let sample = false;

/** Mock mode's tutorial screens, loaded the first time it opens. */
let demo: Record<string, ViewEvent> | null = null;
let demoRoute: Route = { screen: "machine" };

function emitDemo() {
  const e = demo?.[demoRoute.screen];
  if (e) for (const l of listeners) l(e);
}

/** Switches to the tutorial's sample sessions, or back to the real ones. */
export async function setDemo(on: boolean, route: Route): Promise<void> {
  sample = on;
  if (MOCK) {
    if (on && !demo) {
      const files = import.meta.glob<ViewEvent>("../../fixtures/demo/*.json", { import: "default" });
      const entries = await Promise.all(Object.entries(files).map(async ([path, load]) => [path.split("/").pop()!.replace(/\.json$/, ""), await load()] as const));
      demo = Object.fromEntries(entries);
    }
    await setRoute(route);
    return;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_tutorial", { on, route });
}

export async function setRoute(route: Route): Promise<void> {
  if (MOCK) {
    if (sample) {
      demoRoute = route;
      queueMicrotask(emitDemo);
    } else {
      mockRoute = route;
      queueMicrotask(emitMock);
    }
    return;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_route", { route });
}

export async function onView(cb: Listener): Promise<() => void> {
  listeners.push(cb);
  if (MOCK) {
    queueMicrotask(emitMock);
    return () => listeners.splice(listeners.indexOf(cb), 1);
  }
  const { listen } = await import("@tauri-apps/api/event");
  // A view already on its way when the tutorial opens or closes belongs to the other side.
  return listen<ViewEvent>("view", (e) => {
    if (Boolean(e.payload.sample) === sample) cb(e.payload);
  });
}

export async function onNavigate(cb: (r: Route) => void): Promise<() => void> {
  if (MOCK) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<Route>("navigate", (e) => cb(e.payload));
}

/** Tutorial… in the menu bar menu. */
export async function onTutorial(cb: () => void): Promise<() => void> {
  if (MOCK) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen("tutorial", () => cb());
}

/** Runs a backend command and returns its message, or the error text. While the
 * tutorial runs, the backend applies it to the sample sessions. */
export async function command(name: string, args: Record<string, unknown> = {}): Promise<string> {
  if (MOCK) return `${name} (mock)`;
  const { invoke } = await import("@tauri-apps/api/core");
  try {
    return String((await invoke(name, args)) ?? "");
  } catch (e) {
    return String(e);
  }
}
