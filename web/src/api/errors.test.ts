import { describe, expect, it } from "vitest";
import { ApiError, isErrorBody } from "./errors";

describe("ApiError", () => {
  it("carries the code and message of the shared error shape", () => {
    const error = new ApiError(404, { error: { code: "not_found", message: "No such page" } });
    expect(error).toMatchObject({ status: 404, code: "not_found", message: "No such page" });
  });

  it.each([null, "<html>Bad gateway</html>", { error: "boom" }, { error: { code: 1 } }])(
    "falls back to a generic code for a body like %j",
    (body) => {
      const error = new ApiError(502, body);
      expect(error).toMatchObject({ status: 502, code: "http_error", fromServer: false });
      expect(isErrorBody(body)).toBe(false);
    },
  );

  it.each([
    [502, null, true],
    [504, "<html>Gateway timeout</html>", true],
    [502, { error: { code: "bad_gateway", message: "Upstream webhook failed" } }, false],
    [503, null, false],
    [500, null, false],
  ])("reads status %i with body %j as the upstream being down: %s", (status, body, expected) => {
    expect(new ApiError(status, body).upstreamDown).toBe(expected);
  });
});
