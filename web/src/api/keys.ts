/**
 * Query keys. Everything under "account", "admin" and "workspaces" belongs to
 * whoever is signed in, so signing in or out drops those trees; "meta" and
 * "auth" are public and stay.
 *
 * Each workspace's data sits under its own key, `workspace(id)`, so a change
 * to who is in it, or to what they may do there, refreshes all of it at once.
 */
export const keys = {
  meta: ["meta"],
  authConfig: ["auth", "config"],
  invite: (code: string) => ["auth", "invite", code],
  me: ["account", "me"],
  preferences: (userId: string) => ["account", "preferences", userId],
  sessions: ["account", "sessions"],
  tokens: ["account", "tokens"],
  myInvitations: ["account", "invitations"],
  users: ["admin", "users"],
  invites: ["admin", "invites"],
  inviteCheck: (code: string) => ["admin", "invites", "check", code],
  joinRequests: ["admin", "join-requests"],
  settings: ["admin", "settings"],
  adminWorkspaces: ["admin", "workspaces"],
  workspaces: ["workspaces", "list"],
  workspace: (id: string) => ["workspaces", id],
  workspaceDetail: (id: string) => ["workspaces", id, "detail"],
  members: (id: string) => ["workspaces", id, "members"],
  sentInvitations: (id: string) => ["workspaces", id, "invitations"],
  inviteLinks: (id: string) => ["workspaces", id, "invite-links"],
  teamspaces: (id: string) => ["workspaces", id, "teamspaces"],
  teamspace: (id: string, teamspaceId: string) => ["workspaces", id, "teamspace", teamspaceId],
  teamspaceMembers: (id: string, teamspaceId: string) => [
    "workspaces",
    id,
    "teamspace",
    teamspaceId,
    "members",
  ],
  /** Every page of a workspace read one by one, with what hangs under each. */
  pages: (id: string) => ["workspaces", id, "pages"],
  page: (id: string, pageId: string) => ["workspaces", id, "pages", pageId],
  breadcrumbs: (id: string, pageId: string) => ["workspaces", id, "pages", pageId, "breadcrumbs"],
  shares: (id: string, pageId: string) => ["workspaces", id, "pages", pageId, "shares"],
  /** Every tree of a workspace's pages: its teamspaces', the private and the shared one. */
  pageTrees: (id: string) => ["workspaces", id, "page-trees"],
  teamspacePages: (id: string, teamspaceId: string) => [
    "workspaces",
    id,
    "page-trees",
    "teamspace",
    teamspaceId,
  ],
  privatePages: (id: string) => ["workspaces", id, "page-trees", "private"],
  sharedPages: (id: string) => ["workspaces", id, "page-trees", "shared"],
  trash: (id: string) => ["workspaces", id, "trash"],
  favorites: (id: string) => ["workspaces", id, "favorites"],
} as const;

/** The roots of the signed-in user's data. */
export const PRIVATE_ROOTS = ["account", "admin", "workspaces"] as const;
