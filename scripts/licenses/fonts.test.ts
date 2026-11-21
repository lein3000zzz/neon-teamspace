import { describe, expect, it, onTestFinished } from "bun:test";
import { fixture, OFL_TEXT } from "./fixture.ts";
import { bundledFonts, fontProblem } from "./fonts.ts";

const tree = (files: Record<string, string>) => fixture(files, onTestFinished);

describe("bundledFonts", () => {
  it("finds every directory holding font files, at any depth", () => {
    const publicDir = tree({
      "favicon.svg": "<svg/>",
      "fonts/mono/Mono-Regular.woff2": "",
      "fonts/mono/Mono-Bold.woff2": "",
      "fonts/mono/LICENSE.md": OFL_TEXT,
      "deep/er/Serif.ttf": "",
    });
    expect(bundledFonts(publicDir)).toEqual([
      { dir: "deep/er", fonts: ["Serif.ttf"], licenseFiles: [] },
      {
        dir: "fonts/mono",
        fonts: ["Mono-Bold.woff2", "Mono-Regular.woff2"],
        licenseFiles: ["fonts/mono/LICENSE.md"],
      },
    ]);
  });
});

/** The font check's verdict on one font directory with the given files beside the font. */
const check = (files: Record<string, string>) => {
  const publicDir = tree({ "fonts/face/Face.woff2": "", ...files });
  const [font] = bundledFonts(publicDir);
  if (font === undefined) {
    throw new Error("fixture has no font");
  }
  return fontProblem(publicDir, font);
};

describe("fontProblem", () => {
  it("accepts the OFL 1.1 next to the files, under any usual name", () => {
    expect(check({ "fonts/face/OFL.txt": OFL_TEXT })).toBeNull();
    expect(check({ "fonts/face/LICENSE": OFL_TEXT })).toBeNull();
  });

  it("rejects fonts without a license file", () => {
    expect(check({ "fonts/face/README.md": OFL_TEXT })).toMatch("no license file");
  });

  it("rejects any other license", () => {
    expect(check({ "fonts/face/LICENSE": "MIT License" })).toMatch("OFL-1.1");
  });
});
