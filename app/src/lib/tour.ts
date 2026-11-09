// The tutorial: its steps, and what it remembers between launches.
//
// It runs on sample sessions (see scripts/demo-sessions.mjs) that the backend plays
// through the same live source as real ones, so every key and click works. Most steps
// ask you to do something and move on once you have. Wander somewhere a step didn't ask
// for, and the card explains the screen you're on until you come back.
//
// Each step has a fixed id. The app records which ids someone has seen, so a step added
// in a later release shows at the next launch even after "Don't show again". Reword a
// step freely, but give a new step a new id, and never reuse an old one.

import type { Route } from "./types";

/** The sample sessions, agent, and call the steps walk through. */
export const QUESTION = "demo-rate-limits";
export const PERMISSION = "demo-apple-pay";
export const INVOICES = "demo-invoice-perf";
export const AGENT = "a5b7e2c91d04f6a28";
export const CALL = "toolu_demo_013";

/** What the tutorial watches to see whether a step is done. */
export interface Watch {
  route: Route;
  /** Every screen under the current one, the current one last. */
  stack: Route[];
  /** The last key pressed since the step appeared, as `KeyboardEvent.key`. */
  key: string | null;
  /** What's typed in the table's filter. */
  filter: string;
  help: boolean;
}

export interface Tip {
  id: string;
  title: string;
  body: string;
  /** Where Back, Next, and picking up at launch put you. Earlier screens go under it. */
  path: Route[];
  /**
   * The `data-tour` value of what the arrow points at. With a list, the first one on
   * screen. None, or none on screen, puts the card in the corner.
   */
  target?: string | string[];
  /** The screens the step belongs on. Anywhere else, the card explains that screen. */
  on?: (r: Route) => boolean;
  /** What the step asks for. Once it's true, the tutorial moves on by itself. */
  done?: (w: Watch) => boolean;
}

const MAP: Route = { screen: "machine" };
const PUSHING: Route = { screen: "session", id: PERMISSION };
const LANES: Route = { screen: "lanes", id: PERMISSION };
const AGENT_R: Route = { screen: "agent", session: PERMISSION, agent: AGENT };
const CALL_R: Route = { screen: "call", session: PERMISSION, agent: AGENT, call: CALL };

const is = (r: Route, screen: Route["screen"], id?: string) => r.screen === screen && (id === undefined || ("id" in r ? r.id : "session" in r ? r.session : undefined) === id);
const waiting = (r: Route) => is(r, "session", QUESTION) || is(r, "session", PERMISSION);
const key = (w: Watch, ...keys: string[]) => w.key !== null && keys.includes(w.key.toLowerCase());

export const TIPS: Tip[] = [
  {
    id: "map",
    path: [MAP],
    target: "map",
    title: "Every session, live",
    body: "Each box is a Claude Code session, grouped by project, with the sub-agents it spawned underneath. Amber is working, green is finished, and pink is waiting for you. These are sample sessions, and they're running: watch them move. Yours come back when the tutorial closes.",
  },
  {
    id: "needs-you",
    path: [MAP],
    target: "needs-you",
    title: "Who needs you",
    body: "Sessions stuck on a permission prompt or a question line up here, with what they're asking and how long they've waited. Press N to open one.",
    done: (w) => waiting(w.route),
  },
  {
    id: "waiting",
    path: [MAP, PUSHING],
    target: "waiting",
    title: "Waiting on you",
    body: "The top of the session says what it's stuck on: a command it wants permission to run, or a question for you. antfarm only watches, so you answer in the session's own terminal. Press N again for the next one waiting.",
    on: waiting,
    done: (w) => w.stack.some((r) => is(r, "session", QUESTION)) && w.stack.some((r) => is(r, "session", PERMISSION)),
  },
  {
    id: "lanes",
    path: [MAP, PUSHING],
    target: "open-lanes",
    title: "Swimlanes",
    body: "Press L to see the session's agents side by side.",
    on: (r) => r.screen === "session" || r.screen === "lanes",
    done: (w) => w.route.screen === "lanes",
  },
  {
    id: "scrub",
    path: [MAP, PUSHING, LANES],
    target: "lanes",
    title: "Look back",
    body: "One lane per agent, a bar per tool call. Drag the playhead back, or press ←, to see what was running at that moment. Press End to follow now again.",
    on: (r) => r.screen === "lanes",
    done: (w) => key(w, "end"),
  },
  {
    id: "agent",
    path: [MAP, PUSHING],
    target: [`agent-${AGENT}`, `needs-${PERMISSION}`],
    title: "One agent",
    body: "Open apple-pay from the waiting row at the top, then click its agent “Build the Apple Pay button”.",
    done: (w) => w.route.screen === "agent" && w.route.agent === AGENT,
  },
  {
    id: "calls",
    path: [MAP, PUSHING, AGENT_R],
    target: `call-${CALL}`,
    title: "Its calls",
    body: "Every call this agent made, in order, with how long each took. A call still running shows … instead of a time. Click the Write call to open it.",
    on: (r) => r.screen === "agent" && r.agent === AGENT,
    done: (w) => w.route.screen === "call" && w.route.call === CALL,
  },
  {
    id: "call",
    path: [MAP, PUSHING, AGENT_R, CALL_R],
    target: "call-tabs",
    title: "One call",
    body: "The call's input and output. “Why this call” is what the agent said just before making it, which is usually the fastest way to tell what it's up to.",
    on: (r) => r.screen === "call" && r.call === CALL,
  },
  {
    id: "go",
    path: [MAP, PUSHING],
    target: "go",
    title: "Get to the session",
    body: "Press G to jump to the session's tmux pane, editor, or terminal, where you answer it. A sample has nowhere to jump, so here it says where it would go. C copies the command it's waiting on.",
    on: (r) => "session" in r ? r.session === PERMISSION : "id" in r && r.id === PERMISSION,
    done: (w) => key(w, "g"),
  },
  {
    id: "quiet",
    path: [MAP, PUSHING],
    target: "quiet",
    title: "Not now",
    body: "Press Z to snooze apple-pay for ten minutes, or M to mute it. Either way it stops asking for you and drops out of the waiting row on every other screen.",
    on: (r) => "session" in r ? r.session === PERMISSION : "id" in r && r.id === PERMISSION,
    done: (w) => key(w, "z", "m"),
  },
  {
    id: "filter",
    path: [MAP],
    title: "Find a session",
    body: "Press / to switch to the table and filter it. Try typing billing.",
    done: (w) => w.filter.trim().length >= 3,
  },
  {
    id: "jump",
    path: [MAP],
    target: "jump",
    title: "Jump anywhere",
    body: "Press ⌘K and type invoice, then Enter, to open any session, agent, or project by name.",
    done: (w) => is(w.route, "session", INVOICES),
  },
  {
    id: "keys",
    path: [MAP],
    target: "jump",
    title: "Every key",
    body: "Press ? any time for every key. To see this tutorial again, use Tutorial there, in ⌘K, or in the menu bar icon's menu.",
  },
];

/** What the card says on a screen the current step didn't ask for. */
export const SCREENS: Record<Route["screen"], { title: string; body: string }> = {
  machine: { title: "The map", body: "Every session on this machine, grouped by project. Click one to open it, or press / for a table you can filter." },
  project: { title: "A project", body: "Every session in one repository side by side, and the files they touched. Two sessions editing the same file in the same working tree are flagged here: readme-refresh and apple-pay both changed README.md." },
  session: { title: "A session", body: "Its agents, a timeline per agent, the files it changed, and every tool call. Click an agent or a call to open it." },
  lanes: { title: "Swimlanes", body: "One lane per agent, a bar per call. Drag the playhead to look back, and press End to follow now." },
  agent: { title: "An agent", body: "One agent's calls in order. A sub-agent you haven't opened before loads its history when you do." },
  call: { title: "A call", body: "Its input, its output, and why the agent made it. A background command, like invoice-perf's benchmark, shows its output as it arrives." },
};

/** What the app keeps between launches. */
export interface Saved {
  /** "Don't show again" was ticked, or the tour was finished. */
  dismissed: boolean;
  /** The step it was closed on, to pick up there next time. */
  at: string | null;
  /** Every step id this person has been shown. */
  seen: string[];
}

export const EMPTY: Saved = { dismissed: false, at: null, seen: [] };

/** A tutorial in progress: the steps it walks through and which one is up. */
export interface Run {
  tips: Tip[];
  index: number;
  /** "new" when it shows only steps added since the person dismissed it. */
  mode: "tour" | "new";
}

/** What to show when the app starts, if anything. */
export function onLaunch(saved: Saved, tips: Tip[] = TIPS): Run | null {
  if (!saved.dismissed) {
    const i = saved.at ? tips.findIndex((t) => t.id === saved.at) : 0;
    return { tips, index: Math.max(0, i), mode: "tour" };
  }
  const unseen = tips.filter((t) => !saved.seen.includes(t.id));
  return unseen.length ? { tips: unseen, index: 0, mode: "new" } : null;
}

/** Opened from Tutorial: every step, from the first. */
export function fromStart(tips: Tip[] = TIPS): Run {
  return { tips, index: 0, mode: "tour" };
}

/** Whether the person is somewhere the current step didn't ask for. */
export function away(tip: Tip, route: Route): boolean {
  return tip.on ? !tip.on(route) : false;
}

/** Records that the current step was shown. */
export function see(saved: Saved, run: Run): Saved {
  const id = run.tips[run.index]!.id;
  return saved.seen.includes(id) ? saved : { ...saved, seen: [...saved.seen, id] };
}

/**
 * Closing. With "Don't show again" ticked, or after Done on the last step, it stops
 * opening at launch. Otherwise it opens on this step next time.
 */
export function close(saved: Saved, run: Run, dontShow: boolean, done = false): Saved {
  if (dontShow || done) return { ...saved, dismissed: true, at: null };
  if (run.mode === "new") return saved;
  return { ...saved, at: run.tips[run.index]!.id };
}

const KEY = "antfarm.tutorial";

export function load(): Saved {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return EMPTY;
    const v = JSON.parse(raw) as Partial<Saved>;
    return {
      dismissed: v.dismissed === true,
      at: typeof v.at === "string" ? v.at : null,
      seen: Array.isArray(v.seen) ? v.seen.filter((s): s is string => typeof s === "string") : [],
    };
  } catch {
    return EMPTY;
  }
}

export function store(saved: Saved) {
  try {
    localStorage.setItem(KEY, JSON.stringify(saved));
  } catch {
    /* private mode: the tutorial opens again next launch */
  }
}
