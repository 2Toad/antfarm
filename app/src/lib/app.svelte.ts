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

  get route(): Route {
    return this.stack[this.stack.length - 1]!;
  }

  /** The body for the current route, once the backend has sent it. */
  body<T>(screen: string): T | null {
    const b = this.view?.body;
    return b && b.screen === screen ? (b as unknown as T) : null;
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
