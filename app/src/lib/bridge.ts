// The only place the UI talks to the Rust side. In mock mode (`pnpm dev:mock`) it
// replays recorded view events from app/fixtures instead, so screens can be checked in a
// plain browser.

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

export async function setRoute(route: Route): Promise<void> {
  if (MOCK) {
    mockRoute = route;
    queueMicrotask(emitMock);
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
  return listen<ViewEvent>("view", (e) => cb(e.payload));
}

export async function onNavigate(cb: (r: Route) => void): Promise<() => void> {
  if (MOCK) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<Route>("navigate", (e) => cb(e.payload));
}

/** Runs a backend command and returns its message, or the error text. */
export async function command(name: string, args: Record<string, unknown> = {}): Promise<string> {
  if (MOCK) return `${name} (mock)`;
  const { invoke } = await import("@tauri-apps/api/core");
  try {
    return String((await invoke(name, args)) ?? "");
  } catch (e) {
    return String(e);
  }
}
