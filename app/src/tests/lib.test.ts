import { describe, expect, it } from "vitest";
import { categoryOf, glyph } from "../lib/glyph";
import { dur, ms, rate } from "../lib/format";

describe("glyphs", () => {
  it("gives each state its own shape", () => {
    expect(glyph("Waiting")).toBe("◆");
    expect(glyph("Working")).toBe("●");
    expect(glyph("Finished")).toBe("✓");
    expect(glyph("Still")).toBe("○");
    expect(glyph("Exited")).toBe("–");
  });

  it("files tools into families", () => {
    expect(categoryOf("Bash")).toBe("shell");
    expect(categoryOf("Edit")).toBe("edit");
    expect(categoryOf("AskUserQuestion")).toBe("ask");
    expect(categoryOf("mcp__x__y")).toBe("other");
  });
});

describe("formatting matches the Rust side", () => {
  it("formats durations", () => {
    expect(dur(45)).toBe("45s");
    expect(dur(252)).toBe("4m12s");
    expect(dur(3780)).toBe("1h03m");
    expect(dur(187200)).toBe("2d4h");
    expect(dur(null)).toBe("-");
  });

  it("formats milliseconds and rates", () => {
    expect(ms(400)).toBe("0.4s");
    expect(ms(12400)).toBe("12.4s");
    expect(rate(8200)).toBe("8.2k");
    expect(rate(1_500_000)).toBe("1.5M");
  });
});

describe("route bodies", () => {
  it("holds back a body that belongs to the previous route", async () => {
    const { app } = await import("../lib/app.svelte");
    app.stack = [{ screen: "machine" }, { screen: "session", id: "b" }];
    app.view = { body: { screen: "session", id: "a" } } as never;
    expect(app.body("session")).toBeNull();
    app.view = { body: { screen: "session", id: "b" } } as never;
    expect(app.body("session")).not.toBeNull();
    expect(app.body("lanes")).toBeNull();
  });

  it("matches an agent body by its session and agent", async () => {
    const { app } = await import("../lib/app.svelte");
    app.stack = [{ screen: "agent", session: "s1", agent: "a2" }];
    app.view = { body: { screen: "agent", session: "s1", id: "a2" } } as never;
    expect(app.body("agent")).not.toBeNull();
    app.view = { body: { screen: "agent", session: "s1", id: "a1" } } as never;
    expect(app.body("agent")).toBeNull();
    app.view = { body: { screen: "agent", session: "s0", id: "a2" } } as never;
    expect(app.body("agent")).toBeNull();
  });

  it("matches a call body by its session, agent, and call", async () => {
    const { app } = await import("../lib/app.svelte");
    app.stack = [{ screen: "call", session: "s1", agent: "a2", call: "c3" }];
    app.view = { body: { screen: "call", session: "s1", agent: "a2", call: "c3" } } as never;
    expect(app.body("call")).not.toBeNull();
    app.view = { body: { screen: "call", session: "s1", agent: "a1", call: "c3" } } as never;
    expect(app.body("call")).toBeNull();
  });

  it("tells a gone target from one still loading", async () => {
    const { app } = await import("../lib/app.svelte");
    app.stack = [{ screen: "agent", session: "s1", agent: "a2" }];
    app.view = { body: { screen: "session", id: "s1" } } as never;
    expect(app.gone).toBe(false);
    app.view = { body: { screen: "gone", what: "agent" } } as never;
    expect(app.gone).toBe(true);
  });
});
