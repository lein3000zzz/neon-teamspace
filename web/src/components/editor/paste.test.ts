import { describe, expect, it } from "vitest";
import { PASTE_LIMIT, pasteFits } from "./paste";

/** A clipboard holding text of one kind. */
function clipboard(type: string, text: string) {
  return { getData: (asked: string) => (asked === type ? text : "") };
}

describe("what a paste may add", () => {
  it("takes ordinary text, and what the clipboard holds as markup alone", () => {
    expect(pasteFits(clipboard("text/plain", "A paragraph.\n\n- and a list"))).toBe(true);
    expect(pasteFits(clipboard("text/html", "<p>A paragraph.</p>"))).toBe(true);
    expect(pasteFits(clipboard("text/plain", ""))).toBe(true);
  });

  it("counts the text in UTF-8, as Yjs sends it", () => {
    // Three bytes each: past the limit, though fewer characters than it.
    const wide = "あ".repeat(Math.ceil(PASTE_LIMIT / 3) + 1);
    expect(wide.length).toBeLessThan(PASTE_LIMIT);
    expect(pasteFits(clipboard("text/plain", wide))).toBe(false);
  });

  it("counts what each line costs as a block", () => {
    // About 2.4 MiB of text, but a block for every line pushes it past the limit.
    const lines = "Twenty characters..\n".repeat(120_000);
    expect(new TextEncoder().encode(lines).byteLength).toBeLessThan(PASTE_LIMIT);
    expect(pasteFits(clipboard("text/plain", lines))).toBe(false);
    // Blank lines cost nothing.
    expect(pasteFits(clipboard("text/plain", "\n".repeat(PASTE_LIMIT / 2)))).toBe(true);
  });
});
