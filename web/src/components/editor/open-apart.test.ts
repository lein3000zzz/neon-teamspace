import { afterEach, describe, expect, it, vi } from "vitest";
import { openClickedLink } from "./open-apart";

describe("a link clicked in the editor", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  it("opens in a tab the linked page cannot send this one elsewhere from", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const anchor = document.createElement("a");
    anchor.href = "https://elsewhere.example/docs";
    anchor.target = "_blank";
    const words = document.createElement("strong");
    words.textContent = "the docs";
    anchor.append(words);
    document.body.append(anchor);

    const clicked = new MouseEvent("click", { button: 0 });
    Object.defineProperty(clicked, "target", { value: words });

    expect(openClickedLink(clicked)).toBe(true);
    expect(open).toHaveBeenCalledExactlyOnceWith(
      "https://elsewhere.example/docs",
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("opens nothing when the click was on no link", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const clicked = new MouseEvent("click", { button: 0 });
    Object.defineProperty(clicked, "target", { value: document.body });

    expect(openClickedLink(clicked)).toBe(true);
    expect(open).not.toHaveBeenCalled();
  });
});
