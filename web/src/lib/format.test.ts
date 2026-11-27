import { describe, expect, it } from "vitest";
import { formatBytes, formatShortDateTime } from "./format";

describe("sizes", () => {
  it("read in binary units, 1024 apart", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(2 * 1024 ** 3)).toBe("2 GiB");
  });
});

describe("a moment where room is short", () => {
  const now = new Date(2026, 10, 27, 9, 0);

  it("leaves out the year it shares with now", () => {
    const moment = new Date(2026, 1, 3, 12, 30).toISOString();
    expect(formatShortDateTime(moment, now)).not.toContain("2026");
  });

  it("keeps the year of another", () => {
    const moment = new Date(2025, 11, 31, 23, 30).toISOString();
    expect(formatShortDateTime(moment, now)).toContain("2025");
  });
});
