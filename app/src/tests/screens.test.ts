import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";

vi.mock("../lib/bridge", () => ({
  setRoute: vi.fn(async () => {}),
  onView: vi.fn(async () => () => {}),
  onNavigate: vi.fn(async () => () => {}),
  onTutorial: vi.fn(async () => () => {}),
  setDemo: vi.fn(async () => {}),
  command: vi.fn(async (name: string) => `${name} ok`),
}));

import { app } from "../lib/app.svelte";
import { command } from "../lib/bridge";
import type { ViewEvent } from "../lib/types";
import machine from "../../fixtures/machine.json";
import session from "../../fixtures/session.json";
import call from "../../fixtures/call.json";
import agent from "../../fixtures/agent.json";
import NeedsYou from "../components/NeedsYou.svelte";
import Machine from "../screens/Machine.svelte";
import Session from "../screens/Session.svelte";
import Call from "../screens/Call.svelte";
import Agent from "../screens/Agent.svelte";
import Tour from "../components/Tour.svelte";
import Help from "../components/Help.svelte";
import { QUESTION, TIPS, fromStart, load } from "../lib/tour";
import { setDemo } from "../lib/bridge";

beforeEach(() => {
  app.stack = [{ screen: "machine" }];
  app.machineMode = "map";
});

describe("needs-you band", () => {
  it("names the waiting session and its request", async () => {
    app.view = machine as unknown as ViewEvent;
    render(NeedsYou);
    expect(screen.getByLabelText("Waiting for you").textContent).toContain("live-tail");
    expect(screen.getByText("PERMISSION")).toBeTruthy();
    expect(screen.getByText("permission prompt")).toBeTruthy();
  });

  it("go to terminal asks the backend for that session", async () => {
    app.view = machine as unknown as ViewEvent;
    render(NeedsYou);
    await fireEvent.click(screen.getAllByText(/Go to terminal/)[0]!);
    expect(command).toHaveBeenCalledWith("go_to_terminal", { session: "live-tail" });
  });
});

describe("machine screen", () => {
  it("draws a chamber per session with the waiting badge", async () => {
    app.view = machine as unknown as ViewEvent;
    render(Machine);
    expect(screen.getByLabelText("Machine map").textContent).toContain("live-tail");
    expect(screen.getByText("!")).toBeTruthy();
  });

  it("table rows open with the keyboard", async () => {
    app.view = machine as unknown as ViewEvent;
    app.machineMode = "table";
    const r = render(Machine);
    await tick();
    const m = r.component as unknown as { key: (e: KeyboardEvent) => boolean };
    expect(m.key(new KeyboardEvent("keydown", { key: "ArrowDown" }))).toBe(true);
    expect(m.key(new KeyboardEvent("keydown", { key: "Enter" }))).toBe(true);
    expect(app.stack.length).toBe(2);
  });
});

describe("session screen", () => {
  it("shows the waiting panel and the calls", async () => {
    app.view = session as unknown as ViewEvent;
    app.stack = [{ screen: "machine" }, { screen: "session", id: "live-tail" }];
    render(Session);
    expect(screen.getByLabelText("Waiting for you").textContent).toContain("permission request");
    expect(screen.getByText(/tool calls · newest first/i)).toBeTruthy();
  });
});

describe("agent screen", () => {
  it("draws a sub-agent opened from its session", async () => {
    app.stack = [{ screen: "agent", session: "live-tail", agent: "acd22a10701128561" }];
    app.view = agent as unknown as ViewEvent;
    render(Agent);
    expect(screen.queryByText(/is gone/)).toBeNull();
    expect(screen.getByRole("heading").textContent).toBe("Lyqkz i0833 msml vlcvxvzxuu layoj");
    expect(screen.queryByText(/Loading this agent's history/)).toBeNull();
  });

  it("says when an old agent's history is still loading", async () => {
    app.stack = [{ screen: "agent", session: "live-tail", agent: "acd22a10701128561" }];
    app.view = { ...agent, body: { ...agent.body, read: false } } as unknown as ViewEvent;
    render(Agent);
    expect(screen.getByText(/Loading this agent's history/)).toBeTruthy();
  });
});

describe("call screen", () => {
  it("says a running foreground command has no output yet", async () => {
    app.view = call as unknown as ViewEvent;
    render(Call);
    expect(screen.getByText(/no output until the call returns/)).toBeTruthy();
  });

  it("shows the input with secrets masked by the backend", async () => {
    app.view = call as unknown as ViewEvent;
    render(Call);
    await fireEvent.click(screen.getByRole("tab", { name: "Input" }));
    expect(document.querySelector(".out")?.textContent).toContain('"command"');
  });
});

describe("tutorial", () => {
  beforeEach(async () => {
    localStorage.clear();
    if (app.tour) await app.closeTour(false);
    localStorage.clear();
  });

  it("shows the tip count, title, and footer", async () => {
    render(Tour);
    await app.startTour();
    await tick();
    expect(screen.getByText(`Tip 1 of ${TIPS.length}`)).toBeTruthy();
    expect(screen.getByRole("heading").textContent).toBe(TIPS[0]!.title);
    expect((screen.getByRole("button", { name: "Back" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("checkbox", { name: "Don't show again" })).toBeTruthy();
    expect(screen.getByText(/Sample sessions/)).toBeTruthy();
    expect(setDemo).toHaveBeenLastCalledWith(true, { screen: "machine" });
  });

  it("next opens the tip's screen with the screens before it underneath", async () => {
    render(Tour);
    await app.startTour();
    for (let i = 0; i < 4; i++) {
      await fireEvent.click(screen.getByRole("button", { name: /^(Next|Skip)$/ }));
      await tick();
    }
    expect(screen.getByText(`Tip 5 of ${TIPS.length}`)).toBeTruthy();
    expect(app.stack).toEqual(TIPS[4]!.path);
    await fireEvent.click(screen.getByRole("button", { name: "Back" }));
    await tick();
    expect(screen.getByText(`Tip 4 of ${TIPS.length}`)).toBeTruthy();
  });

  it("close puts back the person's own screen and remembers the tip", async () => {
    app.stack = [{ screen: "machine" }, { screen: "session", id: "live-tail" }];
    render(Tour);
    await app.startTour();
    await fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await tick();
    expect(app.tour).toBeNull();
    expect(app.stack).toEqual([{ screen: "machine" }, { screen: "session", id: "live-tail" }]);
    expect(setDemo).toHaveBeenLastCalledWith(false, { screen: "session", id: "live-tail" });
    expect(load()).toMatchObject({ dismissed: false, at: TIPS[1]!.id });
  });

  it("close with Don't show again ticked keeps it shut", async () => {
    render(Tour);
    await app.startTour();
    await tick();
    await fireEvent.click(screen.getByRole("checkbox", { name: "Don't show again" }));
    await fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(load()).toMatchObject({ dismissed: true, at: null });
  });

  it("moves on by itself once the step is done, leaving the person where they are", async () => {
    render(Tour);
    await app.startTour();
    await fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await tick();
    expect(app.tour?.index).toBe(1);
    expect(screen.getByRole("button", { name: "Skip" })).toBeTruthy();
    // What N does: open the session that's waited longest.
    app.go({ screen: "session", id: QUESTION });
    await tick();
    expect(app.tour?.index).toBe(2);
    expect(app.stack).toEqual([{ screen: "machine" }, { screen: "session", id: QUESTION }]);
  });

  it("notes the keys a step asks for, and lets them through", async () => {
    render(Tour);
    await app.startTour(fromStart());
    app.showTip(TIPS.findIndex((t) => t.id === "scrub"));
    await tick();
    const e = new KeyboardEvent("keydown", { key: "End", cancelable: true });
    window.dispatchEvent(e);
    await tick();
    expect(e.defaultPrevented).toBe(false);
    expect(app.tour?.tips[app.tour.index]!.id).toBe("agent");
  });

  it("explains the screen when the person wanders off, with a way back", async () => {
    render(Tour);
    await app.startTour();
    const i = TIPS.findIndex((t) => t.id === "waiting");
    app.showTip(i);
    app.go({ screen: "project", key: "/Users/demo/code/billing-api" });
    await tick();
    expect(screen.getByRole("heading").textContent).toBe("A project");
    expect(app.tour?.index).toBe(i);
    await fireEvent.click(screen.getByRole("button", { name: `Back to tip ${i + 1}` }));
    await tick();
    expect(app.stack).toEqual(TIPS[i]!.path);
    expect(screen.getByRole("heading").textContent).toBe(TIPS[i]!.title);
  });

  it("opens from the help overlay", async () => {
    app.help = true;
    render(Help);
    await fireEvent.click(screen.getByRole("button", { name: "Tutorial" }));
    expect(app.help).toBe(false);
    expect(app.tour?.index).toBe(0);
  });
});
