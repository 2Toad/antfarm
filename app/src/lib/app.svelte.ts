// App-wide state: the latest view, where the person is, and transient messages.

import { command, onNavigate, onTutorial, onView, setDemo, setRoute } from "./bridge";
import * as tour from "./tour";
import type { Route, ViewEvent } from "./types";

class AppState {
  view = $state<ViewEvent | null>(null);
  stack = $state<Route[]>([{ screen: "machine" }]);
  machineMode = $state<"map" | "table">("map");
  theme = $state<"system" | "dark" | "light">("system");
  toast = $state<{ text: string; until: number } | null>(null);
  palette = $state(false);
  help = $state(false);
  /** The tutorial, while it is open. */
  tour = $state<tour.Run | null>(null);
  /** The last key pressed since the current step appeared, for steps that ask for one. */
  tourKey = $state<string | null>(null);
  /** What's typed in the table's filter, for the step that asks for it. */
  tourFilter = $state("");
  /** Where the person was before the tutorial took over the screen. */
  private beforeTour: { stack: Route[]; mode: "map" | "table" } | null = null;
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
    // Session screens name their session `id`; agent and call screens name it `session`,
    // and the agent screen's body names its agent `id`.
    const want = r.session ?? r.id;
    const got = b.session ?? b.id;
    if (want != null && got != null && want !== got) return null;
    const gotAgent = b.agent ?? (b.screen === "agent" ? b.id : undefined);
    if (r.agent != null && gotAgent != null && r.agent !== gotAgent) return null;
    if (r.call != null && b.call != null && r.call !== b.call) return null;
    if (r.key != null && b.key != null && r.key !== b.key) return null;
    return b as unknown as T;
  }

  /** The backend says the current route's session, agent, or call no longer exists. */
  get gone(): boolean {
    return (this.view?.body as { screen?: string } | null | undefined)?.screen === "gone";
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

  /** Opens the tutorial on its sample sessions. */
  async startTour(run: tour.Run = tour.fromStart()) {
    this.help = false;
    this.palette = false;
    const fresh = !this.tour;
    if (fresh) this.beforeTour = { stack: this.stack, mode: this.machineMode };
    this.tour = run;
    const tip = run.tips[run.index]!;
    this.stack = tip.path;
    this.machineMode = "map";
    this.view = null;
    if (fresh) await setDemo(true, this.route);
    this.showTip(run.index);
  }

  /**
   * Moves the tutorial to step `index`. Back, Next, and "Back to tip" also put the
   * screen where the step starts; a step done by hand leaves it where the person is.
   */
  showTip(index: number, move = true) {
    if (!this.tour) return;
    this.tour = { ...this.tour, index };
    this.tourKey = null;
    tour.store(tour.see(tour.load(), this.tour));
    if (move) {
      this.stack = this.tour.tips[index]!.path;
      setRoute(this.route);
    }
  }

  /** The current step is done: on to the next one, from where the person is if it fits there. */
  advance() {
    if (!this.tour || this.tour.index >= this.tour.tips.length - 1) return;
    const next = this.tour.index + 1;
    this.showTip(next, !tour.fits(this.tour.tips[next]!, this.route));
  }

  /** Closes the tutorial and puts back the person's own sessions and screen. */
  async closeTour(dontShow: boolean, done = false) {
    if (!this.tour) return;
    tour.store(tour.close(tour.load(), this.tour, dontShow, done));
    this.tour = null;
    this.view = null;
    this.stack = this.beforeTour?.stack ?? [{ screen: "machine" }];
    this.machineMode = this.beforeTour?.mode ?? "map";
    this.beforeTour = null;
    this.tourFilter = "";
    await setDemo(false, this.route);
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
    // A notification or menu click leaves the tutorial for the session it names.
    await onNavigate(async (r) => {
      await this.closeTour(false);
      this.go(r);
    });
    await onTutorial(() => this.startTour());
    await setRoute(this.route);
    // A route in the URL is a screenshot or a test, which the tutorial would cover.
    const run = initial ? null : tour.onLaunch(tour.load());
    if (run) await this.startTour(run);
  }
}

export const app = new AppState();
