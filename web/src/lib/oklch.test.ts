import { describe, expect, it } from "vitest";
import { oklchToHex } from "./oklch";

describe("oklchToHex", () => {
  it("gives the sRGB primaries and the ends of the grey scale", () => {
    // CSS Color 4's OKLCH of sRGB red, green and blue.
    expect(oklchToHex(0.627955, 0.257683, 29.2339)).toBe("#ff0000");
    expect(oklchToHex(0.86644, 0.294827, 142.4953)).toBe("#00ff00");
    expect(oklchToHex(0.452014, 0.313214, 264.052)).toBe("#0000ff");
    expect(oklchToHex(1, 0, 0)).toBe("#ffffff");
    expect(oklchToHex(0, 0, 0)).toBe("#000000");
  });

  it("clips colours outside sRGB rather than overflowing", () => {
    expect(oklchToHex(0.9, 0.4, 145)).toMatch(/^#[0-9a-f]{6}$/);
  });
});
