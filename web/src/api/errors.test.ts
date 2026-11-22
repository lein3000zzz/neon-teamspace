import { describe, expect, it } from "vitest";
import { ApiError, hasCode, isErrorBody, parseRetryAfter } from "./errors";

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

describe("ApiError details", () => {
  it("keeps the problems the server listed per field", () => {
    const error = new ApiError(400, {
      error: {
        code: "invalid_settings",
        message: "nothing was saved",
        problems: [{ key: "auth.min_password_length", message: "must be between 8 and 128" }],
      },
    });
    expect(error.problemFor("auth.min_password_length")).toBe("must be between 8 and 128");
    expect(error.problemFor("instance.name")).toBeUndefined();
  });

  it("reads Retry-After from the response, in whole seconds", () => {
    const response = new Response(null, { status: 429, headers: { "Retry-After": "30" } });
    expect(ApiError.from(response, null).retryAfter).toBe(30);
  });

  it.each([null, "", "soon", "Wed, 21 Oct 2026 07:28:00 GMT", "-5", "1.5"])(
    "reads Retry-After %j as no hint",
    (header) => {
      expect(parseRetryAfter(header)).toBeUndefined();
    },
  );

  it("tells an ended session from a wrong password", () => {
    const ended = new ApiError(401, {
      error: { code: "unauthenticated", message: "sign in first" },
    });
    const wrong = new ApiError(401, {
      error: { code: "invalid_credentials", message: "invalid email or password" },
    });
    expect(ended.sessionEnded).toBe(true);
    expect(wrong.sessionEnded).toBe(false);
    expect(hasCode(wrong, "invalid_credentials", "user_disabled")).toBe(true);
    expect(hasCode(new Error("boom"), "invalid_credentials")).toBe(false);
  });
});
