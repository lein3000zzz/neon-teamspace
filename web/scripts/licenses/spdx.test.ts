import { describe, expect, it } from "vitest";
import { declaredLicense, parseSpdx, satisfies } from "./spdx.ts";

const allowed = new Set(["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception"]);
const ok = (text: string) => satisfies(parseSpdx(text), allowed);

describe("satisfies", () => {
  it.each([
    ["MIT", true],
    ["GPL-3.0-only", false],
    ["(MIT OR GPL-3.0-only)", true],
    ["MIT AND GPL-3.0-only", false],
    ["MIT AND Apache-2.0", true],
    ["Apache-2.0 WITH LLVM-exception", true],
    ["MIT WITH LLVM-exception", false],
    ["GPL-2.0-only OR MIT AND Apache-2.0", true],
    ["(GPL-2.0-only OR MIT) AND AGPL-3.0-only", false],
  ])("%s is allowed: %s", (text, expected) => {
    expect(ok(text)).toBe(expected);
  });

  it.each([
    ["SEE LICENSE IN LICENSE.md", "trailing"],
    ["(MIT", "unexpected end"],
    ["MIT OR", "unexpected end"],
    ["MIT )", "trailing"],
    ["MIT OR OR", 'unexpected "OR"'],
    ["", "unexpected end"],
  ])("rejects the malformed expression %j", (text, message) => {
    expect(() => parseSpdx(text)).toThrow(message);
  });
});

describe("declaredLicense", () => {
  it("reads the modern field", () => {
    expect(declaredLicense({ license: "MIT" })).toBe("MIT");
  });

  it("joins the legacy array as alternatives", () => {
    expect(declaredLicense({ licenses: [{ type: "MIT" }, { type: "Apache-2.0" }] })).toBe(
      "(MIT) OR (Apache-2.0)",
    );
  });

  it("reads the legacy object form", () => {
    expect(declaredLicense({ license: { type: "ISC" } })).toBe("(ISC)");
  });

  it("returns undefined when nothing is declared", () => {
    expect(declaredLicense({})).toBeUndefined();
  });
});
