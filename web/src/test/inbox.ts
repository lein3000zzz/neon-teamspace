import { http, HttpResponse, type HttpResponseResolver } from "msw";
import type { MailPreference, Notification, NotificationKind } from "@/api/inbox";
import type { Page } from "@/api/pages";
import type { Person } from "@/api/workspaces";
import { apiError } from "./fixtures";
import { inboxLink, server } from "./server";
import type { Recorded } from "./workspaces";

/** Fixed ids for the notifications of the tests, the newest last. */
export const NOTIFICATION_IDS = {
  first: "0193a5b2-0000-7000-8000-0000000001a1",
  second: "0193a5b2-0000-7000-8000-0000000001a2",
  third: "0193a5b2-0000-7000-8000-0000000001a3",
} as const;

/** The token of a notification mail's link, a low-entropy stand-in for the signed one. */
export const UNSUBSCRIBE_TOKEN = `nu_${"u".repeat(40)}`;

const DESIGN = {
  id: "0193a5b2-0000-7000-8000-00000000a001",
  name: "Design",
  slug: "design",
  hue: 200,
} as const;

type Seed = {
  id: string;
  kind: NotificationKind;
  actor?: Person;
  page?: Page;
  thread?: string;
  comment?: string;
  block?: string;
  excerpt?: string;
  read?: boolean;
  archived?: boolean;
};

/** A notification as the server lists it, in Design. */
export function notification(seed: Seed, createdAt = "2026-11-25T09:00:00Z"): Notification {
  return {
    id: seed.id,
    kind: seed.kind,
    workspace: DESIGN,
    created_at: createdAt,
    ...(seed.actor === undefined ? {} : { actor: seed.actor }),
    ...(seed.page === undefined ? {} : { page: seed.page }),
    ...(seed.thread === undefined ? {} : { thread_id: seed.thread }),
    ...(seed.comment === undefined ? {} : { comment_id: seed.comment }),
    ...(seed.block === undefined ? {} : { block_id: seed.block }),
    ...(seed.excerpt === undefined ? {} : { excerpt: seed.excerpt }),
    ...(seed.read ? { read_at: "2026-11-25T10:00:00Z" } : {}),
    ...(seed.archived ? { archived_at: "2026-11-25T11:00:00Z" } : {}),
  };
}

function unread(each: Notification): boolean {
  return (each.read_at ?? null) === null && (each.archived_at ?? null) === null;
}

type Options = {
  notifications?: Notification[];
  mail?: MailPreference;
  mailAvailable?: boolean;
};

/**
 * The inbox routes of the server, holding what changes, so the inbox reads
 * back what it did, newest first. Every request that changes something is
 * recorded. `tell()` says on the live feed that the inbox changed.
 */
export function fakeInbox(options: Options = {}) {
  const state = {
    notifications: options.notifications ?? [],
    mail: options.mail ?? ("each" as MailPreference),
    mailAvailable: options.mailAvailable ?? true,
    /** Whether the unsubscribe link's token is one the server signed. */
    tokenValid: true,
  };
  const changes: Recorded[] = [];

  const record = (
    resolver: (body: unknown, params: Record<string, string>, url: URL) => Response,
  ) =>
    (async ({ request, params }) => {
      const text = await request.text();
      const body: unknown = text === "" ? undefined : JSON.parse(text);
      const url = new URL(request.url);
      changes.push({ method: request.method, path: url.pathname + url.search, body });
      return resolver(body, params as Record<string, string>, url);
    }) satisfies HttpResponseResolver;

  const settings = () => ({
    mail: state.mail,
    mail_available: state.mailAvailable,
    digest_interval_secs: 3600,
  });

  server.use(
    http.get("/api/v1/inbox", ({ request }) => {
      const filter = new URL(request.url).searchParams.get("filter") ?? "unread";
      const listed = state.notifications.filter((each) => {
        const archived = (each.archived_at ?? null) !== null;
        if (filter === "archived") {
          return archived;
        }
        return !archived && (filter === "all" || unread(each));
      });
      return HttpResponse.json({ notifications: listed.toReversed() });
    }),
    http.get("/api/v1/inbox/count", () =>
      HttpResponse.json({ unread: state.notifications.filter(unread).length }),
    ),
    http.patch(
      "/api/v1/notifications/:id",
      record((body, params) => {
        const found = state.notifications.find((each) => each.id === params["id"]);
        if (found === undefined) {
          return apiError(404, "not_found", "no such notification");
        }
        const { read, archived } = body as { read?: boolean; archived?: boolean };
        if (read === true) {
          found.read_at = "2026-11-26T09:00:00Z";
        } else if (read === false) {
          delete found.read_at;
        }
        if (archived === true) {
          found.archived_at = "2026-11-26T09:00:00Z";
        } else if (archived === false) {
          delete found.archived_at;
        }
        return new HttpResponse(null, { status: 204 });
      }),
    ),
    http.post(
      "/api/v1/inbox/read",
      record(() => {
        const marked = state.notifications.filter(unread);
        for (const each of marked) {
          each.read_at = "2026-11-26T09:00:00Z";
        }
        return HttpResponse.json({ marked: marked.length });
      }),
    ),
    http.get("/api/v1/me/notifications", () => HttpResponse.json(settings())),
    http.put(
      "/api/v1/me/notifications",
      record((body) => {
        state.mail = (body as { mail: MailPreference }).mail;
        return HttpResponse.json(settings());
      }),
    ),
    http.post(
      "/api/v1/unsubscribe",
      record((_body, _params, url) => {
        if (!state.tokenValid || url.searchParams.get("token") !== UNSUBSCRIBE_TOKEN) {
          return apiError(400, "invalid_token", "the link is not one this server signed");
        }
        state.mail = "none";
        return new HttpResponse(null, { status: 204 });
      }),
    ),
    inboxLink.addEventListener("connection", ({ client }) => {
      client.send(JSON.stringify({ type: "ready" }));
    }),
  );
  return {
    state,
    changes,
    /** Says on the live feed that the inbox changed, as the server does. */
    tell() {
      inboxLink.broadcast(JSON.stringify({ type: "inbox" }));
    },
  };
}
