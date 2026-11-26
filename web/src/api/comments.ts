import {
  infiniteQueryOptions,
  queryOptions,
  useMutation,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
} from "@tanstack/react-query";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { ApiError } from "./errors";
import { keys } from "./keys";

export type Thread = Schemas["ThreadOut"];
export type ThreadList = Schemas["ThreadList"];
export type Comment = Schemas["CommentOut"];
export type CommentList = Schemas["CommentList"];
export type Anchor = Schemas["AnchorOut"];
export type PostedComment = Schemas["PostedComment"];
export type MentionCandidates = Schemas["MentionCandidates"];
export type MentionPerson = Schemas["MentionPerson"];

/** A run of a comment's inline content, as the server reads its markdown. */
export type InlineRun =
  | { type: "text"; text: string; styles?: Record<string, boolean | string> }
  | {
      type: "link";
      href: string;
      content: { type: "text"; text: string; styles?: Record<string, boolean | string> }[];
    }
  | { type: "mention"; props: { kind: "user" | "page"; target: string; label: string } };

/** A comment's inline content, typed: the API describes it as BlockNote's, loosely. */
export function runsOf(comment: Comment): InlineRun[] {
  return comment.content as unknown as InlineRun[];
}

/** Where a new thread goes: on the whole page, or on a range of its text. */
export type NewThread = { markdown: string; anchor?: Schemas["AnchorIn"] };

/** Threads, or comments of a thread, read at a time: the most the server gives. */
export const COMMENTS_PART = 100;

const pagePath = (id: string) => ({ params: { path: { id } } });

/** Whether a thread is resolved. */
export function isResolved(thread: Thread): boolean {
  return (thread.resolved_at ?? null) !== null;
}

/**
 * A page's open threads, or its resolved ones, oldest first, a part at a time,
 * each with its first comment and its latest, and what the user may do. The
 * page's connection tells of each change, so a list read moments ago is not
 * read again for the panel opening.
 */
export function threadsQuery(workspaceId: string, pageId: string, resolved: boolean) {
  return infiniteQueryOptions({
    queryKey: keys.threads(workspaceId, pageId, resolved),
    queryFn: ({ pageParam, signal }) =>
      call(
        api.GET("/api/v1/pages/{id}/comments", {
          params: {
            path: { id: pageId },
            query: {
              resolved,
              limit: COMMENTS_PART,
              ...(pageParam === undefined ? {} : { after: pageParam }),
            },
          },
          signal,
        }),
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (part) => part.next ?? undefined,
    staleTime: 30 * 1000,
  });
}

/** One thread of a page, as its list has it. */
export function threadQuery(workspaceId: string, pageId: string, threadId: string) {
  return queryOptions({
    queryKey: keys.thread(workspaceId, pageId, threadId),
    queryFn: ({ signal }) =>
      call(api.GET("/api/v1/threads/{id}", { ...pagePath(threadId), signal })),
  });
}

/** A thread's comments after its first, in order, a part at a time: what its list leaves out. */
export function threadCommentsQuery(workspaceId: string, pageId: string, thread: Thread) {
  return infiniteQueryOptions({
    queryKey: keys.threadComments(workspaceId, pageId, thread.id),
    queryFn: ({ pageParam, signal }) =>
      call(
        api.GET("/api/v1/threads/{id}/comments", {
          params: {
            path: { id: thread.id },
            query: {
              limit: COMMENTS_PART,
              ...(pageParam === undefined ? {} : { after: pageParam }),
            },
          },
          signal,
        }),
      ),
    initialPageParam: thread.comments[0]?.id,
    getNextPageParam: (part) => part.next ?? undefined,
  });
}

/** The people and pages a mention on a page may name, whose names hold `q`. */
export function mentionsQuery(workspaceId: string, pageId: string, q: string) {
  return queryOptions({
    queryKey: keys.mentions(workspaceId, pageId, q),
    queryFn: ({ signal }) =>
      call(
        api.GET("/api/v1/pages/{id}/mentions", {
          params: { path: { id: pageId }, query: { q } },
          signal,
        }),
      ),
    staleTime: 30 * 1000,
  });
}

type ThreadParts = InfiniteData<ThreadList, string | undefined>;

/**
 * A list of threads with one of them as it is now: `thread` in its place,
 * oldest first, or gone with `null`. A thread past the parts read so far waits
 * for its part. `opened` is how many threads it opened, -1 when it closed one.
 */
function withThread(
  parts: ThreadParts,
  id: string,
  thread: Thread | null,
  opened: number,
): ThreadParts {
  const pages = parts.pages.map((part) => ({
    ...part,
    open_threads: Math.max(0, part.open_threads + opened),
    threads: part.threads.filter((each) => each.id !== id),
  }));
  if (thread !== null) {
    // Ids are UUIDv7: in order of their text, as the server orders them.
    let at = pages.findIndex((part) => part.threads.some((each) => each.id > thread.id));
    const last = pages.at(-1);
    if (at < 0 && last !== undefined && (last.next ?? null) === null) {
      at = pages.length - 1;
    }
    const part = pages[at];
    if (part !== undefined) {
      part.threads = [...part.threads, thread].toSorted((a, b) => (a.id < b.id ? -1 : 1));
    }
  }
  return { ...parts, pages };
}

/**
 * Puts a thread as it is now into the page's lists of threads, the open or the
 * resolved, or takes it out of both with `null`, without reading the lists
 * again; the rest of its comments, when read, are read again.
 */
export function placeThread(
  queryClient: QueryClient,
  workspaceId: string,
  pageId: string,
  id: string,
  thread: Thread | null,
) {
  const openKey = keys.threads(workspaceId, pageId, false);
  const wasOpen =
    queryClient
      .getQueryData<ThreadParts>(openKey)
      ?.pages.some((part) => part.threads.some((each) => each.id === id)) ?? false;
  const opened = (thread !== null && !isResolved(thread) ? 1 : 0) - (wasOpen ? 1 : 0);
  for (const resolved of [false, true]) {
    queryClient.setQueryData<ThreadParts>(keys.threads(workspaceId, pageId, resolved), (parts) =>
      parts === undefined
        ? parts
        : withThread(
            parts,
            id,
            thread !== null && isResolved(thread) === resolved ? thread : null,
            opened,
          ),
    );
  }
  if (thread === null) {
    queryClient.removeQueries({ queryKey: keys.thread(workspaceId, pageId, id) });
    return;
  }
  queryClient.setQueryData(keys.thread(workspaceId, pageId, id), thread);
  void queryClient.invalidateQueries({ queryKey: keys.threadComments(workspaceId, pageId, id) });
}

/** The latest reading of each thread asked for, so an older answer never wins over a newer. */
const readings = new Map<string, number>();

/**
 * Reads one thread again, as the page's connection says it changed, and puts
 * it in its place in the lists; one gone, or no longer the user's to see,
 * leaves them. Should it not read, every comment of the page is read again.
 */
export async function refreshThread(
  queryClient: QueryClient,
  workspaceId: string,
  pageId: string,
  id: string,
) {
  const reading = (readings.get(id) ?? 0) + 1;
  readings.set(id, reading);
  let thread: Thread | null;
  try {
    thread = await call(api.GET("/api/v1/threads/{id}", pagePath(id)));
  } catch (error) {
    if (!(error instanceof ApiError && error.status === 404)) {
      refreshComments(queryClient, workspaceId, pageId);
      return;
    }
    thread = null;
  }
  if (readings.get(id) === reading) {
    readings.delete(id);
    placeThread(queryClient, workspaceId, pageId, id, thread);
  }
}

/**
 * Reads every comment of a page again: after its connection came back, when
 * what changed meanwhile went untold.
 */
export function refreshComments(queryClient: QueryClient, workspaceId: string, pageId: string) {
  void queryClient.invalidateQueries({ queryKey: keys.comments(workspaceId, pageId) });
}

/** Each change puts the thread it answers with in its place. */
function usePlacedChange<V>(
  workspaceId: string,
  pageId: string,
  mutationFn: (variables: V) => Promise<PostedComment>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: (posted) =>
      placeThread(queryClient, workspaceId, pageId, posted.thread.id, posted.thread),
  });
}

/** Starts a thread on the page, on its whole or on a range of its text. */
export function useStartThread(workspaceId: string, pageId: string) {
  return usePlacedChange(workspaceId, pageId, (body: NewThread) =>
    call(api.POST("/api/v1/pages/{id}/comments", { ...pagePath(pageId), body })),
  );
}

/** Replies in a thread of the page. */
export function useReply(workspaceId: string, pageId: string) {
  return usePlacedChange(
    workspaceId,
    pageId,
    ({ threadId, markdown }: { threadId: string; markdown: string }) =>
      call(
        api.POST("/api/v1/threads/{id}/comments", {
          ...pagePath(threadId),
          body: { markdown },
        }),
      ),
  );
}

/** Changes the text of one of the user's comments. */
export function useEditComment(workspaceId: string, pageId: string) {
  return usePlacedChange(
    workspaceId,
    pageId,
    ({ commentId, markdown }: { commentId: string; markdown: string }) =>
      call(
        api.PATCH("/api/v1/comments/{id}", {
          ...pagePath(commentId),
          body: { markdown },
        }),
      ),
  );
}

/**
 * Deletes a comment, the user's own or, for one who edits the page, anyone's;
 * a thread left without any goes too.
 */
export function useDeleteComment(workspaceId: string, pageId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ commentId }: { commentId: string; threadId: string }) =>
      send(api.DELETE("/api/v1/comments/{id}", pagePath(commentId))),
    onSuccess: (_done, { threadId }) => refreshThread(queryClient, workspaceId, pageId, threadId),
  });
}

/** Deletes a whole thread, for one who edits the page. */
export function useDeleteThread(workspaceId: string, pageId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (threadId: string) => send(api.DELETE("/api/v1/threads/{id}", pagePath(threadId))),
    onSuccess: (_done, threadId) => placeThread(queryClient, workspaceId, pageId, threadId, null),
  });
}

/** Resolves a thread, or reopens it. */
export function useResolveThread(workspaceId: string, pageId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ threadId, resolved }: { threadId: string; resolved: boolean }) =>
      call(
        api.PATCH("/api/v1/threads/{id}", {
          ...pagePath(threadId),
          body: { resolved },
        }),
      ),
    onSuccess: (thread) => placeThread(queryClient, workspaceId, pageId, thread.id, thread),
  });
}

/** Follows the page's comments, or stops. */
export function useFollowPage(workspaceId: string, pageId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (following: boolean) =>
      call(api.PUT("/api/v1/pages/{id}/follow", { ...pagePath(pageId), body: { following } })),
    onSuccess: ({ following }) => {
      for (const resolved of [false, true]) {
        queryClient.setQueryData<ThreadParts>(
          keys.threads(workspaceId, pageId, resolved),
          (parts) =>
            parts === undefined
              ? parts
              : { ...parts, pages: parts.pages.map((part) => ({ ...part, following })) },
        );
      }
    },
  });
}

/** A mention as markdown writes it: `@[Name](user:<id>)` or `@[Title](page:<id>)`. */
export function mentionMarkdown(kind: "user" | "page", id: string, label: string): string {
  const escaped = label.replace(/[\\[\]*_`~<>]/g, (c) => `\\${c}`);
  return `@[${escaped}](${kind}:${id})`;
}
