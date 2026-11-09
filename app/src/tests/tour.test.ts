import { beforeEach, describe, expect, it } from "vitest";
import { AGENT, away, CALL, close, EMPTY, fromStart, INVOICES, load, onLaunch, PERMISSION, QUESTION, SCREENS, see, store, TIPS, type Saved, type Tip, type Watch } from "../lib/tour";
import type { Route, ViewEvent } from "../lib/types";

const demo = import.meta.glob<ViewEvent>("../../fixtures/demo/*.json", { eager: true, import: "default" });
const screen = (name: string) => demo[`../../fixtures/demo/${name}.json`]!;

describe("the tutorial at launch", () => {
  it("opens on the first tip the first time", () => {
    expect(onLaunch(EMPTY)).toEqual({ tips: TIPS, index: 0, mode: "tour" });
  });

  it("picks up on the tip it was closed on", () => {
    const run = { tips: TIPS, index: 4, mode: "tour" as const };
    const saved = close(EMPTY, run, false);
    expect(saved.at).toBe(TIPS[4]!.id);
    expect(onLaunch(saved)?.index).toBe(4);
  });

  it("starts over if the tip it was closed on no longer exists", () => {
    expect(onLaunch({ ...EMPTY, at: "gone" })?.index).toBe(0);
  });

  it("stays shut after Don't show again", () => {
    const run = fromStart();
    let saved: Saved = EMPTY;
    for (let i = 0; i < TIPS.length; i++) saved = see(saved, { ...run, index: i });
    saved = close(saved, { ...run, index: 2 }, true);
    expect(saved.dismissed).toBe(true);
    expect(onLaunch(saved)).toBeNull();
  });

  it("stays shut after Done on the last tip", () => {
    const run = { ...fromStart(), index: TIPS.length - 1 };
    let saved: Saved = EMPTY;
    for (let i = 0; i < TIPS.length; i++) saved = see(saved, { ...run, index: i });
    expect(onLaunch(close(saved, run, false, true))).toBeNull();
  });

  it("shows only tips added since it was dismissed", () => {
    const added: Tip = { id: "added", path: [{ screen: "machine" }], title: "New", body: "A new tip." };
    const seenAll: Saved = { dismissed: true, at: null, seen: TIPS.map((t) => t.id) };
    expect(onLaunch(seenAll, [...TIPS, added])).toEqual({ tips: [added], index: 0, mode: "new" });
  });

  it("keeps offering new tips that were never reached", () => {
    const a: Tip = { id: "a", path: [{ screen: "machine" }], title: "A", body: "" };
    const b: Tip = { id: "b", path: [{ screen: "machine" }], title: "B", body: "" };
    let saved: Saved = { dismissed: true, at: null, seen: TIPS.map((t) => t.id) };
    const run = onLaunch(saved, [...TIPS, a, b])!;
    saved = close(see(saved, run), run, false);
    expect(onLaunch(saved, [...TIPS, a, b])?.tips).toEqual([b]);
  });

  it("starts from the first tip when opened from Tutorial", () => {
    expect(fromStart().index).toBe(0);
    expect(fromStart().tips).toHaveLength(TIPS.length);
  });
});

describe("what the tutorial stores", () => {
  beforeEach(() => localStorage.clear());

  it("reads back what it wrote", () => {
    const saved: Saved = { dismissed: true, at: "lanes", seen: ["map", "lanes"] };
    store(saved);
    expect(load()).toEqual(saved);
  });

  it("treats anything unreadable as a first launch", () => {
    localStorage.setItem("antfarm.tutorial", "{not json");
    expect(load()).toEqual(EMPTY);
    localStorage.setItem("antfarm.tutorial", JSON.stringify({ dismissed: "yes", seen: [1, "map"] }));
    expect(load()).toEqual({ dismissed: false, at: null, seen: ["map"] });
  });
});

describe("steps and the sample sessions", () => {
  const machine = screen("machine");
  const session = screen("session").body as unknown as { id: string; agents: { id: string }[] };
  const agent = screen("agent").body as unknown as { id: string; calls: { call: string; tool: string }[] };
  const watch = (route: Route, more: Partial<Watch> = {}): Watch => ({ route, stack: [{ screen: "machine" }, route], key: null, filter: "", help: false, ...more });
  const step = (id: string) => TIPS.find((t) => t.id === id)!;

  it("every step id is unique", () => {
    expect(new Set(TIPS.map((t) => t.id)).size).toBe(TIPS.length);
  });

  it("one session waits on a question and one on a permission prompt", () => {
    expect(machine.header.needs_you.map((n) => [n.session, n.kind])).toEqual([
      [QUESTION, "question"],
      [PERMISSION, "permission"],
    ]);
  });

  it("the agent, call, and session the steps name are in the samples", () => {
    expect(session.id).toBe(PERMISSION);
    expect(session.agents.map((a) => a.id)).toContain(AGENT);
    expect(agent.id).toBe(AGENT);
    expect(agent.calls.find((c) => c.call === CALL)?.tool).toBe("Write");
    expect(machine.header.sessions.map((s) => s.id)).toContain(INVOICES);
  });

  it("each step that asks for something is done by exactly that", () => {
    const S = (id: string): Route => ({ screen: "session", id });
    // N can land on either waiting session first, depending on what the map has selected.
    expect(step("needs-you").done!(watch(S(QUESTION)))).toBe(true);
    expect(step("needs-you").done!(watch(S(PERMISSION)))).toBe(true);
    expect(step("needs-you").done!(watch({ screen: "machine" }))).toBe(false);
    expect(step("waiting").done!(watch(S(PERMISSION)))).toBe(false);
    expect(step("waiting").done!(watch(S(QUESTION), { stack: [{ screen: "machine" }, S(PERMISSION), S(QUESTION)] }))).toBe(true);
    expect(step("lanes").done!(watch({ screen: "lanes", id: QUESTION }))).toBe(true);
    expect(step("scrub").done!(watch({ screen: "lanes", id: PERMISSION }, { key: "ArrowLeft" }))).toBe(false);
    expect(step("scrub").done!(watch({ screen: "lanes", id: PERMISSION }, { key: "End" }))).toBe(true);
    expect(step("agent").done!(watch({ screen: "agent", session: PERMISSION, agent: AGENT }))).toBe(true);
    expect(step("calls").done!(watch({ screen: "call", session: PERMISSION, agent: AGENT, call: CALL }))).toBe(true);
    expect(step("go").done!(watch(S(PERMISSION), { key: "G" }))).toBe(true);
    expect(step("quiet").done!(watch(S(PERMISSION), { key: "m" }))).toBe(true);
    expect(step("filter").done!(watch({ screen: "machine" }, { filter: "bil" }))).toBe(true);
    expect(step("jump").done!(watch(S(INVOICES)))).toBe(true);
  });

  it("a step done by hand starts where the step before it leaves you", () => {
    // Each step's path ends on a screen it belongs on, so Back, Next, and resuming
    // never land somewhere the card would call wandering.
    for (const t of TIPS) expect(away(t, t.path[t.path.length - 1]!), t.id).toBe(false);
  });

  it("wandering is any screen the step doesn't belong on", () => {
    expect(away(step("waiting"), { screen: "machine" })).toBe(true);
    expect(away(step("waiting"), { screen: "session", id: QUESTION })).toBe(false);
    expect(away(step("map"), { screen: "call", session: "x", agent: "y", call: "z" })).toBe(false);
    for (const k of ["machine", "project", "session", "lanes", "agent", "call"] as const) expect(SCREENS[k].title).toBeTruthy();
  });
});
