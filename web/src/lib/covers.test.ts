import { describe, expect, it } from "vitest";
import { coverImage, COVERS } from "./covers";

describe("covers", () => {
  it("has every preset the server takes, once", () => {
    expect(new Set(COVERS).size).toBe(14);
  });

  it("draws each from the palette's accent lightness and chroma, at the documented hues", () => {
    expect(coverImage("red")).toBe(
      "linear-gradient(120deg, oklch(var(--accent-l) var(--accent-c) 25), oklch(var(--accent-l) var(--accent-c) 25))",
    );
    expect(coverImage("sunrise")).toBe(
      "linear-gradient(120deg, oklch(var(--accent-l) var(--accent-c) 55), oklch(var(--accent-l) var(--accent-c) 345))",
    );
    expect(coverImage("accent-gradient")).toContain("calc(var(--accent-h) + 180)");
    for (const cover of COVERS) {
      expect(coverImage(cover)).toMatch(/^linear-gradient\(120deg, oklch\(var\(--accent-l\)/);
    }
  });
});
