/**
 * Query keys. Everything under "account" and "admin" belongs to whoever is
 * signed in, so signing in or out drops those two trees; "meta" and "auth" are
 * public and stay.
 */
export const keys = {
  meta: ["meta"],
  authConfig: ["auth", "config"],
  invite: (code: string) => ["auth", "invite", code],
  me: ["account", "me"],
  preferences: (userId: string) => ["account", "preferences", userId],
  sessions: ["account", "sessions"],
  tokens: ["account", "tokens"],
  users: ["admin", "users"],
  invites: ["admin", "invites"],
  inviteCheck: (code: string) => ["admin", "invites", "check", code],
  joinRequests: ["admin", "join-requests"],
  settings: ["admin", "settings"],
} as const;

/** The roots of the signed-in user's data. */
export const PRIVATE_ROOTS = ["account", "admin"] as const;
