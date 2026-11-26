import { queryOptions, useMutation, useQueryClient } from "@tanstack/react-query";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type Thread = Schemas["ThreadOut"];
export type ThreadList = Schemas["ThreadList"];
export type Comment = Schemas["CommentOut"];
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

const pagePath = (id: string) => ({ params: { path: { id } } });

/** A page's threads of comments, oldest first, with what the user may do with them. */
export function commentsQuery(workspaceId: string, pageId: string) {
  return queryOptions({
    queryKey: keys.comments(workspaceId, pageId),
    queryFn: ({ signal }) =>
      call(api.GET("/api/v1/pages/{id}/comments", { ...pagePath(pageId), signal })),
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

/** Reads a page's comments again: after a change here, or one the page's connection told of. */
export function refreshComments(
  queryClient: ReturnType<typeof useQueryClient>,
  workspaceId: string,
  pageId: string,
) {
  void queryClient.invalidateQueries({ queryKey: keys.comments(workspaceId, pageId) });
}

/** Every change to a page's comments reads them again once it is saved. */
function useCommentChange<V, T>(
  workspaceId: string,
  pageId: string,
  mutationFn: (variables: V) => Promise<T>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: () => refreshComments(queryClient, workspaceId, pageId),
  });
}

/** Starts a thread on the page, on its whole or on a range of its text. */
export function useStartThread(workspaceId: string, pageId: string) {
  return useCommentChange(workspaceId, pageId, (body: NewThread) =>
    call(api.POST("/api/v1/pages/{id}/comments", { ...pagePath(pageId), body })),
  );
}

/** Replies in a thread of the page. */
export function useReply(workspaceId: string, pageId: string) {
  return useCommentChange(
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
  return useCommentChange(
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

/** Deletes one of the user's comments; a thread left without any goes too. */
export function useDeleteComment(workspaceId: string, pageId: string) {
  return useCommentChange(workspaceId, pageId, (commentId: string) =>
    send(api.DELETE("/api/v1/comments/{id}", pagePath(commentId))),
  );
}

/** Resolves a thread, or reopens it. */
export function useResolveThread(workspaceId: string, pageId: string) {
  return useCommentChange(
    workspaceId,
    pageId,
    ({ threadId, resolved }: { threadId: string; resolved: boolean }) =>
      call(
        api.PATCH("/api/v1/threads/{id}", {
          ...pagePath(threadId),
          body: { resolved },
        }),
      ),
  );
}

/** Follows the page's comments, or stops. */
export function useFollowPage(workspaceId: string, pageId: string) {
  return useCommentChange(workspaceId, pageId, (following: boolean) =>
    call(api.PUT("/api/v1/pages/{id}/follow", { ...pagePath(pageId), body: { following } })),
  );
}

/** A mention as markdown writes it: `@[Name](user:<id>)` or `@[Title](page:<id>)`. */
export function mentionMarkdown(kind: "user" | "page", id: string, label: string): string {
  const escaped = label.replace(/[\\[\]*_`~<>]/g, (c) => `\\${c}`);
  return `@[${escaped}](${kind}:${id})`;
}
