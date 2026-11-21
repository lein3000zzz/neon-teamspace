import type { Schemas } from "./client";

/** Statuses a reverse proxy (Caddy, the Vite dev proxy) sends when the server behind it is down. */
const GATEWAY_STATUSES: ReadonlySet<number> = new Set([502, 504]);

/**
 * A non-2xx answer from the API. Every error body the server sends has the
 * shape {"error": {"code", "message"}}; anything else (a proxy's HTML page, an
 * empty body) is reported with a generic code so callers handle one type.
 */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  /** False when something other than the server answered, such as a proxy's error page. */
  readonly fromServer: boolean;

  constructor(status: number, body: unknown) {
    const detail = isErrorBody(body) ? body.error : null;
    super(detail?.message ?? `Request failed with status ${status}`);
    this.name = "ApiError";
    this.status = status;
    this.code = detail?.code ?? "http_error";
    this.fromServer = detail !== null;
  }

  /**
   * A proxy in front answered with a gateway error of its own, so the server
   * behind it is down or not running. A gateway status that carries the API's
   * error body came from the server and does not count.
   */
  get upstreamDown(): boolean {
    return !this.fromServer && GATEWAY_STATUSES.has(this.status);
  }
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
    typeof error.message === "string"
  );
}
