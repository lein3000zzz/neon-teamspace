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
  connectedApps: ["account", "connected-apps"],
  /** A client's request on the consent page, as the person signed in may answer it. */
  consent: (request: object) => ["account", "consent", request],
  myInvitations: ["account", "invitations"],
  /** The inbox, every part and the count, which a change to it reads again. */
  inbox: ["account", "inbox"],
  inboxList: (filter: string) => ["account", "inbox", "list", filter],
  inboxCount: ["account", "inbox", "count"],
  notificationSettings: ["account", "notification-settings"],
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
  agents: (id: string) => ["workspaces", id, "agents"],
  agentTokens: (id: string, agentId: string) => ["workspaces", id, "agents", agentId, "tokens"],
  agentTeamspaces: (id: string, agentId: string) => [
    "workspaces",
    id,
    "agents",
    agentId,
    "teamspaces",
  ],
  workspaceApps: (id: string) => ["workspaces", id, "connected-apps"],
  webhooks: (id: string) => ["workspaces", id, "webhooks"],
  webhookDeliveries: (id: string, webhookId: string) => [
    "workspaces",
    id,
    "webhooks",
    webhookId,
    "deliveries",
  ],
  incomingWebhooks: (id: string) => ["workspaces", id, "incoming-webhooks"],
  incomingRequests: (id: string, hookId: string) => [
    "workspaces",
    id,
    "incoming-webhooks",
    hookId,
    "requests",
  ],
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
  comments: (id: string, pageId: string) => ["workspaces", id, "pages", pageId, "comments"],
  /** A page's open threads, or its resolved ones, a part at a time. */
  threads: (id: string, pageId: string, resolved: boolean) => [
    "workspaces",
    id,
    "pages",
    pageId,
    "comments",
    "threads",
    resolved ? "resolved" : "open",
  ],
  /** One thread of a page; with what follows it, its comments a part at a time. */
  thread: (id: string, pageId: string, threadId: string) => [
    "workspaces",
    id,
    "pages",
    pageId,
    "comments",
    "thread",
    threadId,
  ],
  threadComments: (id: string, pageId: string, threadId: string) => [
    "workspaces",
    id,
    "pages",
    pageId,
    "comments",
    "thread",
    threadId,
    "comments",
  ],
  mentions: (id: string, pageId: string, q: string) => [
    "workspaces",
    id,
    "pages",
    pageId,
    "mentions",
    q,
  ],
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
  /** The pages changed last, which the quick search offers before anything is typed. */
  recentPages: (id: string) => ["workspaces", id, "recent-pages"],
  /** A search's results, by what it asked; every page change may change them. */
  search: (id: string, asked: object) => ["workspaces", id, "search", asked],
  /** A page's history, its versions, and each compared with the page as it is. */
  versions: (id: string, pageId: string) => ["workspaces", id, "pages", pageId, "versions"],
  version: (id: string, pageId: string, versionId: string) => [
    "workspaces",
    id,
    "pages",
    pageId,
    "versions",
    versionId,
  ],
  comparison: (id: string, pageId: string, versionId: string) => [
    "workspaces",
    id,
    "pages",
    pageId,
    "versions",
    versionId,
    "compare",
  ],
  /** A database's schema and views, with the rows each view lists under it. */
  database: (id: string, databaseId: string) => ["workspaces", id, "databases", databaseId],
  /** Every view's rows of a database, which a change to any row reads again. */
  databaseRows: (id: string, databaseId: string) => [
    "workspaces",
    id,
    "databases",
    databaseId,
    "rows",
  ],
  viewRows: (id: string, databaseId: string, viewId: string) => [
    "workspaces",
    id,
    "databases",
    databaseId,
    "rows",
    viewId,
  ],
  /** One row with its values, as a row's page shows them. */
  rows: (id: string) => ["workspaces", id, "rows"],
  row: (id: string, rowId: string) => ["workspaces", id, "rows", rowId],
} as const;

/** The roots of the signed-in user's data. */
export const PRIVATE_ROOTS = ["account", "admin", "workspaces"] as const;
