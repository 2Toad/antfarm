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
