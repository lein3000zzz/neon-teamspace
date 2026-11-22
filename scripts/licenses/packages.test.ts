import { describe, expect, it, onTestFinished } from "bun:test";
import { fixture, manifest } from "./fixture.ts";
import { type Installed, installedPackages, judge } from "./packages.ts";
import type { Policy } from "./policy.ts";

describe("installedPackages", () => {
  it("finds scoped packages and nested copies, and skips tool state", () => {
    const project = fixture(
      {
        "node_modules/plain/package.json": manifest("plain"),
        "node_modules/@scope/pkg/package.json": manifest("@scope/pkg", "ISC"),
        "node_modules/plain/node_modules/nested/package.json": manifest("nested", "MIT", "2.0.0"),
        "node_modules/.cache/thing/package.json": manifest("thing"),
        "node_modules/not-a-package/README.md": "",
      },
      onTestFinished,
    );

    expect(installedPackages(project).toSorted((a, b) => a.path.localeCompare(b.path))).toEqual([
      { name: "@scope/pkg", version: "1.0.0", license: "ISC", path: "node_modules/@scope/pkg" },
      { name: "plain", version: "1.0.0", license: "MIT", path: "node_modules/plain" },
      {
        name: "nested",
        version: "2.0.0",
        license: "MIT",
        path: "node_modules/plain/node_modules/nested",
      },
    ]);
  });
});

const allowed = new Set(["MIT", "Apache-2.0"]);

function pkg(name: string, license: string | undefined, version = "1.0.0"): Installed {
  return { name, version, license, path: `node_modules/${name}` };
}

function policy(entries: Partial<Policy>): Policy {
  return { exceptions: [], clarifications: [], ...entries };
}

describe("judge", () => {
  it("allows what the allow list allows and names the rest", () => {
    const verdict = judge(
      [pkg("ok", "MIT OR GPL-3.0-only"), pkg("gpl", "GPL-3.0-only"), pkg("bare", undefined)],
      allowed,
      policy({}),
    );

    expect(verdict.failures).toEqual([
      "gpl@1.0.0 (node_modules/gpl): license GPL-3.0-only",
      "bare@1.0.0 (node_modules/bare): declares no license",
    ]);
    expect(verdict.unused).toEqual([]);
  });

  it("reports a license expression it cannot read", () => {
    const verdict = judge([pkg("odd", "SEE LICENSE IN LICENSE.md")], allowed, policy({}));

    expect(verdict.failures).toEqual([expect.stringContaining("odd@1.0.0")]);
  });

  it("allows an exception only with its exact license", () => {
    const exceptions = [{ name: "data", license: "CC-BY-4.0", reason: "build only" }];

    expect(judge([pkg("data", "CC-BY-4.0")], allowed, policy({ exceptions }))).toMatchObject({
      failures: [],
      unused: [],
      excepted: ["data@1.0.0: CC-BY-4.0"],
    });
    expect(judge([pkg("data", "CC-BY-SA-4.0")], allowed, policy({ exceptions }))).toMatchObject({
      failures: ["data@1.0.0 (node_modules/data): license CC-BY-SA-4.0"],
      unused: ["exception data: CC-BY-4.0"],
    });
  });

  it("matches a name prefix ending in *", () => {
    const exceptions = [{ name: "@img/native-*", license: "LGPL-3.0-or-later", reason: "unused" }];
    const verdict = judge(
      [pkg("@img/native-linux-x64", "LGPL-3.0-or-later"), pkg("@img/other", "LGPL-3.0-or-later")],
      allowed,
      policy({ exceptions }),
    );

    expect(verdict.excepted).toEqual(["@img/native-linux-x64@1.0.0: LGPL-3.0-or-later"]);
    expect(verdict.failures).toEqual([
      "@img/other@1.0.0 (node_modules/@img/other): license LGPL-3.0-or-later",
    ]);
  });

  it("fails on an exception nothing needs", () => {
    const exceptions = [{ name: "gone", license: "CC-BY-4.0", reason: "build only" }];

    expect(judge([pkg("ok", "MIT")], allowed, policy({ exceptions })).unused).toEqual([
      "exception gone: CC-BY-4.0",
    ]);
  });

  it("judges one version by its clarified license", () => {
    const clarifications = [
      { name: "quiet", version: "2.1.0", license: "MIT", reason: "no license field" },
    ];
    const verdict = judge(
      [pkg("quiet", undefined, "2.1.0"), pkg("quiet", undefined, "3.0.0")],
      allowed,
      policy({ clarifications }),
    );

    expect(verdict.clarified).toEqual(["quiet@2.1.0: MIT"]);
    expect(verdict.failures).toEqual(["quiet@3.0.0 (node_modules/quiet): declares no license"]);
    expect(verdict.counts).toEqual([
      ["MIT", 1],
      ["none", 1],
    ]);
  });

  it("checks a clarified license against the allow list too", () => {
    const clarifications = [
      { name: "quiet", version: "1.0.0", license: "GPL-3.0-only", reason: "no license field" },
    ];

    expect(judge([pkg("quiet", undefined)], allowed, policy({ clarifications })).failures).toEqual([
      "quiet@1.0.0 (node_modules/quiet): license GPL-3.0-only",
    ]);
  });

  it("fails on a clarification nothing needs", () => {
    const clarifications = [
      { name: "quiet", version: "2.1.0", license: "MIT", reason: "no license field" },
    ];

    expect(
      judge([pkg("quiet", "MIT", "2.2.0")], allowed, policy({ clarifications })).unused,
    ).toEqual(["clarification quiet@2.1.0"]);
  });
});
