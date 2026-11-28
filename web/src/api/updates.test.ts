import { describe, expect, it } from "vitest";
import { reportedVersion } from "./updates";

describe("reportedVersion", () => {
  it("is a release candidate's release, which its server states", () => {
    expect(reportedVersion("0.3.0-rc.1")).toBe("0.3.0");
    expect(reportedVersion("1.0.0-rc.0")).toBe("1.0.0");
    expect(reportedVersion("0.3.0-rc.12")).toBe("0.3.0");
  });

  it("is any other version itself", () => {
    for (const version of ["0.2.1", "0.3.0-beta.1", "0.3.0-rc.01", "0.3.0-rc.1.2", "0.3.0-rc"]) {
      expect(reportedVersion(version)).toBe(version);
    }
  });
});
