import { describe, expect, it } from "vitest";
import { describeUserAgent, formatWait } from "./format";
import { inviteLink, safeNext } from "./links";

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
  ])("goes to %j as %j", (next, expected) => {
    expect(safeNext(next)).toBe(expected);
  });
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
