import type { Schemas } from "./client";

/** Statuses a reverse proxy (Caddy, the Vite dev proxy) sends when the server behind it is down. */
const GATEWAY_STATUSES: ReadonlySet<number> = new Set([502, 504]);

export type Problem = Schemas["ProblemDetail"];

/**
 * A non-2xx answer from the API. Every error body the server sends has the
 * shape {"error": {"code", "message", "problems"?}}; anything else (a proxy's
 * HTML page, an empty body) is reported with a generic code so callers handle
 * one type.
 */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  /** False when something other than the server answered, such as a proxy's error page. */
  readonly fromServer: boolean;
  /** What was wrong with each field, when the server listed them. */
  readonly problems: readonly Problem[];
  /** Whole seconds to wait before trying again, from a 429's Retry-After header. */
  readonly retryAfter: number | undefined;

  constructor(status: number, body: unknown, retryAfter?: number) {
    const detail = isErrorBody(body) ? body.error : null;
    super(detail?.message ?? `Request failed with status ${status}`);
    this.name = "ApiError";
    this.status = status;
    this.code = detail?.code ?? "http_error";
    this.fromServer = detail !== null;
    this.problems = detail?.problems ?? [];
    this.retryAfter = retryAfter;
  }

  /** An error from a response and the body openapi-fetch read from it. */
  static from(response: Response, body: unknown): ApiError {
    return new ApiError(
      response.status,
      body,
      parseRetryAfter(response.headers.get("Retry-After")),
    );
  }

  /**
   * A proxy in front answered with a gateway error of its own, so the server
   * behind it is down or not running. A gateway status that carries the API's
   * error body came from the server and does not count.
   */
  get upstreamDown(): boolean {
    return !this.fromServer && GATEWAY_STATUSES.has(this.status);
  }

  /**
   * The request carried no working session or token: the cookie expired or was
   * revoked. A wrong password at sign-in is a 401 too, with its own code, and
   * does not count.
   */
  get sessionEnded(): boolean {
    return this.status === 401 && this.code === "unauthenticated";
  }

  /** The problem the server named for one field, if any. */
  problemFor(key: string): string | undefined {
    return this.problems.find((problem) => problem.key === key)?.message;
  }
}

/**
 * Retry-After in whole seconds. The server always sends a number; the HTTP-date
 * form, which only proxies would send, is read as no hint.
 */
export function parseRetryAfter(header: string | null): number | undefined {
  if (header === null || !/^\d+$/.test(header.trim())) {
    return undefined;
  }
  return Number(header.trim());
}

export function isErrorBody(body: unknown): body is Schemas["ErrorBody"] {
  if (typeof body !== "object" || body === null || !("error" in body)) {
    return false;
  }
  const { error } = body;
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    typeof error.code === "string" &&
    "message" in error &&
    typeof error.message === "string" &&
    (!("problems" in error) || Array.isArray(error.problems))
  );
}

/** Whether an error is the API answering with this code. */
export function hasCode(error: unknown, ...codes: string[]): error is ApiError {
  return error instanceof ApiError && codes.includes(error.code);
}
