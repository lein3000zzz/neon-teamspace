import { join, resolve } from "node:path";
import { describe, expect, it, onTestFinished } from "vitest";
import { fixture, manifest, OFL_TEXT } from "../../../scripts/licenses/fixture.ts";
import { collectNotices, COPIED, type CopiedCode, cssImports, packageRoot } from "./notices.ts";

/** The web app's own root, where COPIED names its files from. */
const webRoot = resolve(import.meta.dirname, "../..");

describe("packageRoot", () => {
  it.each([
    ["/app/node_modules/react/index.js", "/app/node_modules/react"],
    [
      "/app/node_modules/@radix-ui/react-slot/dist/index.mjs",
      "/app/node_modules/@radix-ui/react-slot",
    ],
    ["/app/node_modules/a/node_modules/b/lib/b.js", "/app/node_modules/a/node_modules/b"],
    ["/app/node_modules/react/index.js?commonjs-entry", "/app/node_modules/react"],
    ["/app/src/main.tsx", undefined],
    ["\0vite/preload-helper.js", undefined],
    ["/app/node_modules/@scope", undefined],
  ])("%s belongs to %s", (id, root) => {
    expect(packageRoot(id)).toBe(root);
  });
});

describe("cssImports", () => {
  it("reads every form of @import", () => {
    const css = `@import "tailwindcss";\n@import './fonts.css' layer(base);\n@import url("x.css");`;
    expect(cssImports(css)).toEqual(["tailwindcss", "./fonts.css", "x.css"]);
  });
});

/** A small app: one bundled package, two imported stylesheet packages and a font. */
const app = (files: Record<string, string>) =>
  fixture(
    {
      "public/fonts/face/Face.woff2": "",
      "public/fonts/face/OFL.txt": OFL_TEXT,
      "src/index.css": `@import "styles-pkg";\n@import "./more.css";`,
      "src/more.css": `@import "@scope/more-styles/theme.css";\n@import "https://example.com/x.css";`,
      "node_modules/styles-pkg/package.json": manifest("styles-pkg"),
      "node_modules/styles-pkg/LICENSE": "styles-pkg license",
      "node_modules/@scope/more-styles/package.json": manifest("@scope/more-styles", "ISC"),
      "node_modules/@scope/more-styles/LICENSE.md": "more-styles license",
      "node_modules/code/package.json": manifest("code", "MIT OR Apache-2.0"),
      "node_modules/code/LICENSE-MIT": "code MIT license",
      "node_modules/code/LICENSE-APACHE": "code Apache license",
      ...files,
    },
    onTestFinished,
  );

/** Notices for the app built by `app`, with its one bundled module. */
const collect = (
  root: string,
  supplied: Record<string, string> = {},
  copied: readonly CopiedCode[] = [],
) =>
  collectNotices({
    root,
    moduleIds: [join(root, "node_modules/code/index.js"), join(root, "src/main.tsx")],
    stylesheets: [join(root, "src/index.css")],
    publicDir: join(root, "public"),
    supplied,
    suppliedDir: join(root, "supplied"),
    copied,
  });

describe("collectNotices", () => {
  it("lists bundled code, imported stylesheets and fonts with their license texts", () => {
    const text = collect(app({}));
    expect(text).toContain("code 1.0.0\nLicense: MIT OR Apache-2.0");
    expect(text).toContain("code Apache license\n\ncode MIT license");
    expect(text).toContain("styles-pkg 1.0.0\nLicense: MIT");
    expect(text).toContain("@scope/more-styles 1.0.0\nLicense: ISC");
    expect(text).toContain("Font files in fonts/face/\nLicense: OFL-1.1");
    expect(text).toContain(OFL_TEXT);
  });

  it("fails on a bundled package without license text", () => {
    const root = app({ "node_modules/bare/package.json": manifest("bare") });
    expect(() =>
      collectNotices({
        root,
        moduleIds: [join(root, "node_modules/bare/index.js")],
        stylesheets: [],
        publicDir: join(root, "public"),
        supplied: {},
        suppliedDir: join(root, "supplied"),
        copied: [],
      }),
    ).toThrow("No license text in");
  });

  it("carries a package's notice files after its license texts, and fails one with notices alone", () => {
    const text = collect(
      app({
        "node_modules/code/NOTICE.txt": "code took its data from data-pkg, under MIT",
        "node_modules/code/NOTICES": "code's further notices",
      }),
    );
    expect(text).toContain(
      "code Apache license\n\ncode MIT license\n\n" +
        "code took its data from data-pkg, under MIT\n\ncode's further notices",
    );

    const root = app({
      "node_modules/bare/package.json": manifest("bare"),
      "node_modules/bare/NOTICE": "bare's notice",
    });
    expect(() =>
      collectNotices({
        root,
        moduleIds: [join(root, "node_modules/bare/index.js")],
        stylesheets: [],
        publicDir: join(root, "public"),
        supplied: {},
        suppliedDir: join(root, "supplied"),
        copied: [],
      }),
    ).toThrow("No license text in");
  });

  it("uses a supplied text for a package that ships none", () => {
    const root = app({
      "node_modules/bare/package.json": manifest("bare"),
      "supplied/bare.LICENSE": "bare license from its repository",
    });
    const text = collectNotices({
      root,
      moduleIds: [join(root, "node_modules/bare/index.js")],
      stylesheets: [],
      publicDir: join(root, "public"),
      supplied: { bare: "bare.LICENSE" },
      suppliedDir: join(root, "supplied"),
      copied: [],
    });
    expect(text).toContain("bare 1.0.0\nLicense: MIT\n" + "-".repeat(80));
    expect(text).toContain("bare license from its repository");
  });

  it("fails on a supplied text that is no longer needed", () => {
    expect(() => collect(app({}), { gone: "gone.LICENSE" })).toThrow("no longer has: gone");
    expect(() => collect(app({}), { code: "code.LICENSE" })).toThrow("now ships its own license");
  });

  it("always lists the shadcn/ui code copied into src/components/ui, with its MIT notice", () => {
    const text = collectNotices({
      root: webRoot,
      moduleIds: [],
      stylesheets: [],
      publicDir: join(app({}), "public"),
      supplied: {},
      suppliedDir: join(webRoot, "scripts/licenses/supplied"),
      copied: COPIED,
    });
    expect(text).toContain(
      "shadcn/ui, the code copied into src/components/ui/\nLicense: MIT\n" + "-".repeat(80),
    );
    expect(text).toContain("Copyright (c) 2023 shadcn");
    expect(text).toContain("The above copyright notice and this permission notice shall be");
  });

  it("fails on copied code whose license text is missing", () => {
    const copied = [{ name: "lib", dir: "src/lib", license: "MIT", file: "src/lib/lib.LICENSE" }];
    expect(() => collect(app({}), {}, copied)).toThrow(
      "src/lib: src/lib/lib.LICENSE is missing: copy lib's license from its repository",
    );
    const text = collect(app({ "src/lib/lib.LICENSE": "lib's license" }), {}, copied);
    expect(text).toContain("lib, the code copied into src/lib/\nLicense: MIT");
    expect(text).toContain("lib's license");
  });

  it("says where to get the source of an MPL-2.0 package, and no other", () => {
    const root = app({
      "node_modules/code/package.json": JSON.stringify({
        name: "code",
        version: "2.1.0",
        license: "MPL-2.0",
        repository: { type: "git", url: "git+https://example.com/code.git", directory: "pkg" },
      }),
    });
    const text = collect(root);
    expect(text).toContain(
      [
        "code 2.1.0",
        "License: MPL-2.0",
        "Source: https://www.npmjs.com/package/code/v/2.1.0",
        "Repository: https://example.com/code (pkg)",
        "-".repeat(80),
      ].join("\n"),
    );
    expect(text).toContain("styles-pkg 1.0.0\nLicense: MIT\n" + "-".repeat(80));
    expect(text).toContain("The source code of each package under MPL-2.0");
  });

  it("fails on a font without its license", () => {
    expect(() => collect(app({ "public/fonts/other/Other.woff2": "" }))).toThrow(
      "fonts/other: no license file",
    );
  });
});
