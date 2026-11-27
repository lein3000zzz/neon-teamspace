import { afterEach, describe, expect, it, vi } from "vitest";
import { describeUserAgent, formatWait } from "./format";
import { inviteLink, openApart, safeNext } from "./links";

describe("the page after signing in", () => {
  it.each([
    ["/account/tokens?tab=1#new", "/account/tokens?tab=1#new"],
    ["/", "/"],
    [undefined, "/"],
    ["account", "/"],
    ["//evil.example/path", "/"],
    ["/\\evil.example", "/"],
    ["/\t/evil.example", "/"],
    ["https://evil.example/", "/"],
    ["/login?next=%2Faccount", "/"],
    ["/.//evil.example/x", "/"],
    ["/%2e//evil.example", "/"],
    ["/a/..//evil.example", "/"],
    ["/./\\evil.example", "/"],
    ["/docs/./guide", "/docs/guide"],
  ])("goes to %j as %j", (next, expected) => {
    expect(safeNext(next)).toBe(expected);
  });
});

describe("addresses a page opens", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("open in a tab cut off from this one", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    openApart("https://elsewhere.example/docs?a=1");
    openApart("/api/v1/files/abc");
    openApart("mailto:ada@example.com");
    expect(open.mock.calls).toEqual([
      ["https://elsewhere.example/docs?a=1", "_blank", "noopener,noreferrer"],
      [`${window.location.origin}/api/v1/files/abc`, "_blank", "noopener,noreferrer"],
      ["mailto:ada@example.com", "_blank", "noopener,noreferrer"],
    ]);
  });

  it.each(["javascript:alert(1)", " JavaScript:alert(1)", "data:text/html,hi", "http://["])(
    "open nothing for %j",
    (href) => {
      const open = vi.spyOn(window, "open").mockReturnValue(null);
      openApart(href);
      expect(open).not.toHaveBeenCalled();
    },
  );
});

describe("invite links", () => {
  it("carry the code to the sign-up form, escaped", () => {
    expect(inviteLink("inv_a-b_c")).toBe(`${window.location.origin}/register?invite=inv_a-b_c`);
    expect(inviteLink("a&b")).toContain("invite=a%26b");
  });
});

describe("words for a wait", () => {
  it.each([
    [0, "1 second"],
    [1, "1 second"],
    [30, "30 seconds"],
    [60, "1 minute"],
    [90, "2 minutes"],
    [3600, "1 hour"],
    [5400, "2 hours"],
  ])("says %i seconds as %j", (seconds, words) => {
    expect(formatWait(seconds)).toBe(words);
  });
});

describe("browsers named from their User-Agent", () => {
  it.each([
    [
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36 Edg/131.0",
      "Edge on Windows",
    ],
    [
      "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
      "Safari on iOS",
    ],
    ["Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0", "Firefox on Linux"],
    ["curl/8.10.1", "curl"],
    ["", undefined],
  ])("names %j", (agent, name) => {
    expect(describeUserAgent(agent)).toBe(name);
  });
});
