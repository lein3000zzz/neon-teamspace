import { http, HttpResponse, type HttpResponseResolver } from "msw";
import type { Comment, InlineRun, MentionPerson, Thread } from "@/api/comments";
import type { Page } from "@/api/pages";
import type { Person } from "@/api/workspaces";
import { ada, apiError } from "./fixtures";
import { server } from "./server";
import type { Recorded } from "./workspaces";

/** Fixed ids for the threads and comments of the tests, and for those they make, in order. */
export const COMMENT_IDS = {
  thread: "0193a5b2-0000-7000-8000-0000000000e1",
  first: "0193a5b2-0000-7000-8000-0000000000f1",
  reply: "0193a5b2-0000-7000-8000-0000000000f2",
} as const;

const MADE_THREADS = [
  "0193a5b2-0000-7000-8000-0000000000e7",
  "0193a5b2-0000-7000-8000-0000000000e8",
  "0193a5b2-0000-7000-8000-0000000000e9",
] as const;

const MADE_COMMENTS = [
  "0193a5b2-0000-7000-8000-0000000000f7",
  "0193a5b2-0000-7000-8000-0000000000f8",
  "0193a5b2-0000-7000-8000-0000000000f9",
] as const;

const CREATED = "2026-11-24T09:00:00Z";
const LATER = "2026-11-25T09:00:00Z";

const MENTION = /@\[((?:\\.|[^\]\\])*)\]\((user|page):([^)\s]+)\)/g;

/** A comment's markdown as the server reads it: its text, and its mentions as mentions. */
export function runsFrom(markdown: string): InlineRun[] {
  const runs: InlineRun[] = [];
  let at = 0;
  for (const found of markdown.matchAll(MENTION)) {
    if (found.index > at) {
      runs.push({ type: "text", text: markdown.slice(at, found.index) });
    }
    const [, label = "", kind, target = ""] = found;
    runs.push({
      type: "mention",
      props: {
        kind: kind === "page" ? "page" : "user",
        target,
        label: label.replaceAll(/\\(.)/g, "$1"),
      },
    });
    at = found.index + found[0].length;
  }
  if (at < markdown.length) {
    runs.push({ type: "text", text: markdown.slice(at) });
  }
  return runs;
}

/** A comment as the server sends it. */
export function comment(
  seed: { id: string; thread: string; markdown: string; author?: Person; edited?: boolean },
  createdAt = CREATED,
): Comment {
  return {
    id: seed.id,
    thread_id: seed.thread,
    markdown: seed.markdown,
    content: runsFrom(seed.markdown) as unknown as Comment["content"],
    created_at: createdAt,
    author: seed.author ?? { id: ada.id, kind: "human", display_name: "Ada", hue: ada.hue },
    ...(seed.edited ? { edited_at: LATER } : {}),
  };
}

/** A thread on a page as a whole, of one comment by `author`. */
export function pageThread(
  pageId: string,
  markdown: string,
  author?: Person,
  id: string = COMMENT_IDS.thread,
): Thread {
  return {
    id,
    page_id: pageId,
    created_at: CREATED,
    created_by: author?.id ?? ada.id,
    comments: [
      comment({ id: COMMENT_IDS.first, thread: id, markdown, ...(author ? { author } : {}) }),
    ],
  };
}

type Options = {
  threads?: Thread[];
  mayComment?: boolean;
  following?: boolean;
  /** Who a mention may name, and whether each sees the page. */
  people?: MentionPerson[];
  pages?: Page[];
  /** Those a new comment's mentions do not reach, as the server tells. */
  unnotified?: Person[];
};

/**
 * The comment routes of the server for one page or more, holding what
 * changes, so the panel reads back what it did. Every request that changes
 * something is recorded. Call after fakePages(), whose routes these sit beside.
 */
export function fakeComments(options: Options = {}) {
  const state = {
    threads: options.threads ?? [],
    mayComment: options.mayComment ?? true,
    following: options.following ?? false,
  };
  const changes: Recorded[] = [];
  /** Each search for mentions, by what was typed after `@`. */
  const searches: string[] = [];
  let threadsMade = 0;
  let commentsMade = 0;

  const record = (
    resolver: (body: unknown, params: Record<string, string>) => Response | Promise<Response>,
  ) =>
    (async ({ request, params }) => {
      const text = await request.text();
      const body: unknown = text === "" ? undefined : JSON.parse(text);
      changes.push({ method: request.method, path: new URL(request.url).pathname, body });
      return resolver(body, params as Record<string, string>);
    }) satisfies HttpResponseResolver;

  const newComment = (thread: string, markdown: string): Comment => {
    const id = MADE_COMMENTS[commentsMade] ?? MADE_COMMENTS[0];
    commentsMade += 1;
    return comment({ id, thread, markdown }, LATER);
  };
  const posted = (thread: Thread, commentId: string, status = 201) =>
    HttpResponse.json(
      { thread, comment_id: commentId, unnotified: options.unnotified ?? [] },
      { status },
    );
  const findComment = (id: string) => {
    for (const thread of state.threads) {
      const index = thread.comments.findIndex((each) => each.id === id);
      if (index >= 0) {
        return { thread, index };
      }
    }
    return undefined;
  };

  server.use(
    http.get("/api/v1/pages/:id/comments", ({ params }) =>
      HttpResponse.json({
        threads: state.threads.filter((thread) => thread.page_id === params["id"]),
        may_comment: state.mayComment,
        following: state.following,
      }),
    ),
    http.get("/api/v1/pages/:id/mentions", ({ request }) => {
      const q = new URL(request.url).searchParams.get("q") ?? "";
      searches.push(q);
      const has = (name: string) => name.toLowerCase().includes(q.toLowerCase());
      return HttpResponse.json({
        people: (options.people ?? []).filter((each) => has(each.person.display_name)),
        pages: (options.pages ?? []).filter((each) => has(each.title)),
      });
    }),
    http.post(
      "/api/v1/pages/:id/comments",
      record((body, params) => {
        const { markdown, anchor } = body as {
          markdown: string;
          anchor?: { start: string; end: string; quote: string };
        };
        const id = MADE_THREADS[threadsMade] ?? MADE_THREADS[0];
        threadsMade += 1;
        const first = newComment(id, markdown);
        const thread: Thread = {
          id,
          page_id: params["id"] ?? "",
          created_at: LATER,
          created_by: ada.id,
          comments: [first],
          ...(anchor === undefined ? {} : { anchor: { ...anchor, detached: false } }),
        };
        state.threads = [...state.threads, thread];
        return posted(thread, first.id);
      }),
    ),
    http.post(
      "/api/v1/threads/:id/comments",
      record((body, params) => {
        const thread = state.threads.find((each) => each.id === params["id"]);
        if (thread === undefined) {
          return apiError(404, "not_found", "no such thread");
        }
        const reply = newComment(thread.id, (body as { markdown: string }).markdown);
        thread.comments = [...thread.comments, reply];
        return posted(thread, reply.id);
      }),
    ),
    http.patch(
      "/api/v1/threads/:id",
      record((body, params) => {
        const thread = state.threads.find((each) => each.id === params["id"]);
        if (thread === undefined) {
          return apiError(404, "not_found", "no such thread");
        }
        if ((body as { resolved: boolean }).resolved) {
          thread.resolved_at = LATER;
          thread.resolved_by = ada.id;
        } else {
          delete thread.resolved_at;
          delete thread.resolved_by;
        }
        return HttpResponse.json(thread);
      }),
    ),
    http.patch(
      "/api/v1/comments/:id",
      record((body, params) => {
        const found = findComment(params["id"] ?? "");
        if (found === undefined) {
          return apiError(404, "not_found", "no such comment");
        }
        const before = found.thread.comments[found.index];
        const markdown = (body as { markdown: string }).markdown;
        const changed: Comment = {
          ...comment(
            { id: before?.id ?? "", thread: found.thread.id, markdown },
            before?.created_at,
          ),
          edited_at: LATER,
        };
        found.thread.comments = found.thread.comments.map((each, at) =>
          at === found.index ? changed : each,
        );
        return posted(found.thread, changed.id, 200);
      }),
    ),
    http.delete(
      "/api/v1/comments/:id",
      record((_body, params) => {
        const found = findComment(params["id"] ?? "");
        if (found === undefined) {
          return apiError(404, "not_found", "no such comment");
        }
        found.thread.comments = found.thread.comments.filter((_each, at) => at !== found.index);
        if (found.thread.comments.length === 0) {
          state.threads = state.threads.filter((each) => each !== found.thread);
        }
        return new HttpResponse(null, { status: 204 });
      }),
    ),
    http.put(
      "/api/v1/pages/:id/follow",
      record((body) => {
        state.following = (body as { following: boolean }).following;
        return HttpResponse.json({ following: state.following });
      }),
    ),
  );
  return { state, changes, searches };
}
