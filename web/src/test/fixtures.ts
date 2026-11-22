import { http, HttpResponse } from "msw";
import type { Me } from "@/api/account";
import type { AuthConfig } from "@/api/auth";
import type { Schemas } from "@/api/client";

/** A member, signed in through a browser session. */
export const ada: Me = {
  id: "0193a5b2-1c4d-7e8f-a1b2-c3d4e5f60718",
  email: "ada@example.com",
  display_name: "Ada",
  hue: 342,
  role: "user",
  email_verified_at: "2026-11-01T10:00:00Z",
  created_at: "2026-11-01T10:00:00Z",
  updated_at: "2026-11-01T10:00:00Z",
  credential: "session",
  scopes: ["read", "write"],
};

/** An admin, signed in through a browser session. */
export const grace: Me = {
  ...ada,
  id: "018f3c1e-7b2a-7c3d-9e4f-5a6b7c8d9e0f",
  email: "grace@example.com",
  display_name: "Grace",
  hue: 265,
  role: "admin",
  scopes: ["read", "write", "admin"],
};

/** The server's defaults: sign-ups by invite, no mail, so nothing mailed is on. */
export const defaultAuthConfig: AuthConfig = {
  registration: "invite",
  min_password_length: 12,
  verify_email: false,
  password_reset: false,
  requests: false,
};

export const defaultPreferences: Schemas["Preferences"] = {
  palette: "nothing",
  mode: "system",
  accent_hue: 250,
  ui_font: "default",
};

type ErrorInit = { problems?: Schemas["ProblemDetail"][]; retryAfter?: number };

/** An answer in the server's error shape. */
export function apiError(status: number, code: string, message: string, init: ErrorInit = {}) {
  return HttpResponse.json(
    { error: { code, message, ...(init.problems ? { problems: init.problems } : {}) } },
    {
      status,
      ...(init.retryAfter === undefined
        ? {}
        : { headers: { "Retry-After": String(init.retryAfter) } }),
    },
  );
}

export function signedInAs(me: Me) {
  return http.get("/api/v1/account/me", () => HttpResponse.json(me));
}

export const signedOut = http.get("/api/v1/account/me", () =>
  apiError(401, "unauthenticated", "sign in first"),
);

export function authConfig(patch: Partial<AuthConfig>) {
  return http.get("/api/v1/auth/config", () =>
    HttpResponse.json({ ...defaultAuthConfig, ...patch }),
  );
}

/**
 * The server's side of a browser session: nobody signed in until a login
 * with the user's address and password, nobody again after logging out.
 * Each sign-in is recorded, with the body it came with.
 */
export function fakeSession(user: Me, password: string) {
  let current: Me | null = null;
  const logins: unknown[] = [];
  const handlers = [
    http.get("/api/v1/account/me", () =>
      current ? HttpResponse.json(current) : apiError(401, "unauthenticated", "sign in first"),
    ),
    http.post("/api/v1/auth/login", async ({ request }) => {
      const body = (await request.json()) as Schemas["LoginRequest"];
      logins.push(body);
      if (body.email.toLowerCase() !== user.email || body.password !== password) {
        return apiError(401, "invalid_credentials", "invalid email or password");
      }
      current = user;
      return HttpResponse.json({
        user,
        expires_at: "2026-11-29T10:00:00Z",
        absolute_expires_at: "2026-12-22T10:00:00Z",
      });
    }),
    http.post("/api/v1/auth/logout", () => {
      current = null;
      return new HttpResponse(null, { status: 204 });
    }),
  ];
  return {
    handlers,
    logins,
    get signedIn() {
      return current !== null;
    },
  };
}
