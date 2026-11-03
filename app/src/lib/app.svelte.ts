// App-wide state: the latest view, where the person is, and transient messages.

import { command, onNavigate, onView, setRoute } from "./bridge";
import type { Route, ViewEvent } from "./types";

class AppState {
  view = $state<ViewEvent | null>(null);
  stack = $state<Route[]>([{ screen: "machine" }]);
  machineMode = $state<"map" | "table">("map");
  theme = $state<"system" | "dark" | "light">("system");
  toast = $state<{ text: string; until: number } | null>(null);
  palette = $state(false);
  help = $state(false);
  /** The table and map choices sent to the backend with `set_prefs`. */
  prefs = $state({ sort: "needs_you" as "needs_you" | "cost" | "name", tree: true, show_finished: false, show_exited: false });

  get route(): Route {
    return this.stack[this.stack.length - 1]!;
  }

  /**
   * The body for the current route, once the backend has sent it. A body for
   * the same kind of screen but a different session is the previous route's,
   * so it is held back until the new one arrives.
   */
  body<T>(screen: string): T | null {
    const b = this.view?.body as (Record<string, unknown> & { screen: string }) | null | undefined;
    if (!b || b.screen !== screen) return null;
    const r = this.route as Record<string, unknown>;
    const want = r.id ?? r.session;
    const got = b.id ?? b.session;
    if (want != null && got != null && want !== got) return null;
    if (r.agent != null && b.agent != null && r.agent !== b.agent) return null;
    if (r.call != null && b.call != null && r.call !== b.call) return null;
    if (r.key != null && b.key != null && r.key !== b.key) return null;
    return b as unknown as T;
  }

  /** Changes table and map preferences, here and in the backend. */
  setPrefs(patch: Partial<AppState["prefs"]>, message: string) {
    this.prefs = { ...this.prefs, ...patch };
    command("set_prefs", { prefs: patch });
    this.say(message);
  }

  go(route: Route) {
    this.stack = [...this.stack, route];
    setRoute(route);
  }

  /** Replaces the current route, for changes within a screen such as a swimlane window. */
  replace(route: Route) {
    this.stack = [...this.stack.slice(0, -1), route];
    setRoute(route);
  }

  back() {
    if (this.stack.length > 1) {
      this.stack = this.stack.slice(0, -1);
      setRoute(this.route);
    }
  }

  home() {
    this.stack = [{ screen: "machine" }];
    setRoute(this.route);
  }

  say(text: string) {
    this.toast = { text, until: Date.now() + 3000 };
    setTimeout(() => {
      if (this.toast && this.toast.until <= Date.now()) this.toast = null;
    }, 3100);
  }

  async run(name: string, args: Record<string, unknown> = {}) {
    this.say(await command(name, args));
  }

  setTheme(t: "system" | "dark" | "light") {
    this.theme = t;
    if (t === "system") document.documentElement.removeAttribute("data-theme");
    else document.documentElement.setAttribute("data-theme", t);
    try {
      localStorage.setItem("antfarm.theme", t);
    } catch {
      /* private mode: the choice lasts until reload */
    }
  }

  async start() {
    try {
      const t = localStorage.getItem("antfarm.theme");
      if (t === "dark" || t === "light") this.setTheme(t);
    } catch {
      /* ignore */
    }
    const params = new URLSearchParams(location.search);
    const initial = params.get("route");
    if (initial) {
      try {
        this.stack = [{ screen: "machine" }, JSON.parse(initial) as Route];
      } catch {
        /* ignore a malformed route */
      }
    }
    if (params.get("mode") === "table") this.machineMode = "table";
    await onView((e) => (this.view = e));
    await onNavigate((r) => this.go(r));
    await setRoute(this.route);
  }
}

export const app = new AppState();
