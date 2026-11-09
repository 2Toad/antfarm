import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";

vi.mock("../lib/bridge", () => ({
  setRoute: vi.fn(async () => {}),
  onView: vi.fn(async () => () => {}),
  onNavigate: vi.fn(async () => () => {}),
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
