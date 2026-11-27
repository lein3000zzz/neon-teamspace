import { describe, expect, it } from "vitest";
import { formatBytes } from "./format";

describe("sizes", () => {
  it("read in binary units, 1024 apart", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(2 * 1024 ** 3)).toBe("2 GiB");
  });
});
