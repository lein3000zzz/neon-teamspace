import { useQuery } from "@tanstack/react-query";
import { Check, MessageSquarePlus, Pencil, RotateCcw, Trash2, X } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import {
  commentsQuery,
  runsOf,
  useDeleteComment,
  useEditComment,
  useFollowPage,
  useReply,
  useResolveThread,
  useStartThread,
  type Comment,
  type PostedComment,
  type Thread,
} from "@/api/comments";
import type { Page } from "@/api/pages";
import type { Person } from "@/api/workspaces";
import type { DraftAnchor } from "@/components/editor/comment-highlights";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { PersonLabel } from "@/components/people/person-label";
import { useMe } from "@/components/shell/signed-in";
import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatDateTime } from "@/lib/format";
import { attribution, type Someone } from "@/lib/people";
import { strings } from "@/lib/strings";
import { CommentComposer } from "./comment-composer";
import { CommentContent } from "./comment-content";
import { UnseenNotice } from "./unseen-notice";

const copy = strings.comments;

type Shown = "unresolved" | "resolved";

export type CommentsPanelProps = {
  page: Page;
  /** Everyone a comment may name, by id; empty for a guest. */
  people: ReadonlyMap<string, Someone>;
  /** The thread open in the panel, highlighted in the page. */
  active: string | null;
  /** A thread to move focus to, as when the keyboard opened it; each request a new object. */
  focusRequest: { threadId: string } | null;
  onActive: (threadId: string | null) => void;
  /** The text selected to start a thread on, waiting for its first comment. */
  draft: DraftAnchor | null;
  onDraft: (draft: DraftAnchor | null) => void;
  /** The text selected in the page, when the page has text to select. */
  selection: (() => DraftAnchor | null) | undefined;
  /** Selects an inline thread's text in the page. */
  onReveal: (thread: Thread) => void;
  onClose: () => void;
};

/**
 * A page's comments, in a panel beside it that leaves the page usable: its
 * open threads or its resolved ones, a thread on the page as a whole or on
 * the text selected, replies, and changing or deleting one's own comments.
 * Escape closes it. Mentions of someone who may not see the page say so, with
 * the way to share it.
 */
export function CommentsPanel({
  page,
  people,
  active,
  focusRequest,
  onActive,
  draft,
  onDraft,
  selection,
  onReveal,
  onClose,
}: CommentsPanelProps) {
  const workspace = useWorkspace();
  const headingId = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const panel = useRef<HTMLElement>(null);
  const comments = useQuery(commentsQuery(workspace.id, page.id));
  const start = useStartThread(workspace.id, page.id);
  const follow = useFollowPage(workspace.id, page.id);
  const [shown, setShown] = useState<Shown>("unresolved");
  const [unseen, setUnseen] = useState<readonly Person[]>([]);
  const [noSelection, setNoSelection] = useState(false);
  const threads = comments.data?.threads ?? [];
  const activeThread = threads.find((thread) => thread.id === active);
  // The thread opened shows in its part of the list, resolved or not.
  const [shownFor, setShownFor] = useState<string | null>(null);
  if (activeThread !== undefined && shownFor !== activeThread.id) {
    setShownFor(activeThread.id);
    setShown(isResolved(activeThread) ? "resolved" : "unresolved");
  }
  const listed = threads.filter((thread) => isResolved(thread) === (shown === "resolved"));
  const mayComment = comments.data?.may_comment ?? false;

  useEffect(() => {
    heading.current?.focus();
  }, []);

  const posted = (result: PostedComment) => {
    setUnseen(result.unnotified);
    onActive(result.thread.id);
  };
  const unseenOne = (person: Person) =>
    setUnseen((current) =>
      current.some((each) => each.id === person.id) ? current : [...current, person],
    );

  // Escape inside closes it, unless a field in it took the key first, as a list of mentions does.
  useEffect(() => {
    const closeOnEscape = (event: globalThis.KeyboardEvent) => {
      const inside = event.target instanceof Node && panel.current?.contains(event.target);
      if (event.key === "Escape" && inside === true && !event.defaultPrevented) {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  const commentOnSelection = () => {
    const anchor = selection?.() ?? null;
    setNoSelection(anchor === null);
    if (anchor !== null) {
      onDraft(anchor);
    }
  };

  return (
    <aside
      ref={panel}
      aria-labelledby={headingId}
      data-comments-panel=""
      className="fixed inset-y-0 right-0 z-30 flex w-[min(24rem,100vw)] flex-col gap-4 overflow-y-auto overscroll-contain border-l border-border bg-raised safe-top-3 safe-right-3 safe-bottom-4 pl-3"
    >
      <div className="flex items-center justify-between gap-2">
        <h2
          id={headingId}
          ref={heading}
          tabIndex={-1}
          className="text-base font-semibold outline-none"
        >
          {copy.label}
        </h2>
        <div className="flex items-center gap-1">
          {comments.data !== undefined && (
            <Button
              variant="outline"
              size="sm"
              aria-pressed={comments.data.following}
              title={copy.followHint}
              disabled={follow.isPending}
              onClick={() => follow.mutate(!comments.data.following)}
            >
              {comments.data.following && <Check aria-hidden />}
              {copy.follow}
            </Button>
          )}
          <Button variant="ghost" size="icon" aria-label={copy.close} onClick={onClose}>
            <X aria-hidden />
          </Button>
        </div>
      </div>
      {follow.isError && <ErrorNotice error={follow.error} />}

      {unseen.length > 0 && (
        <UnseenNotice people={unseen} page={page} onDismiss={() => setUnseen([])} />
      )}

      {comments.isPending && <Loading label={copy.loading} />}
      {comments.isError && (
        <ErrorNotice
          error={comments.error}
          retrying={comments.isFetching}
          onRetry={() => void comments.refetch()}
        />
      )}

      {comments.data !== undefined && !mayComment && (
        <p className="text-sm text-muted-foreground">{copy.readOnly}</p>
      )}
      {mayComment && draft !== null && (
        <section aria-label={copy.onTextLabel} className="grid gap-2">
          <blockquote className="line-clamp-3 border-l-2 border-primary pl-2 text-sm text-muted-foreground">
            {draft.quote}
          </blockquote>
          {start.isError && <ErrorNotice error={start.error} />}
          <CommentComposer
            key={`${draft.start}:${draft.end}`}
            workspaceId={workspace.id}
            pageId={page.id}
            label={copy.onTextLabel}
            submitLabel={copy.send}
            onSubmit={async (markdown) => {
              const result = await start.mutateAsync({ markdown, anchor: draft });
              onDraft(null);
              posted(result);
            }}
            onCancel={() => onDraft(null)}
            onUnseen={unseenOne}
          />
        </section>
      )}
      {mayComment && draft === null && (
        <section aria-label={copy.onPage} className="grid gap-2">
          {start.isError && <ErrorNotice error={start.error} />}
          <CommentComposer
            workspaceId={workspace.id}
            pageId={page.id}
            label={copy.onPageLabel}
            submitLabel={copy.send}
            onSubmit={async (markdown) => posted(await start.mutateAsync({ markdown }))}
            onUnseen={unseenOne}
          />
          {selection !== undefined && (
            <div className="grid gap-1">
              <Button
                variant="ghost"
                size="sm"
                className="justify-self-start"
                // A press must not take the selection it comments on away.
                onMouseDown={(event) => event.preventDefault()}
                onClick={commentOnSelection}
              >
                <MessageSquarePlus aria-hidden />
                {copy.onSelection}
              </Button>
              <p
                className="text-xs text-muted-foreground"
                role={noSelection ? "status" : undefined}
              >
                {noSelection ? copy.selectFirst : copy.shortcut}
              </p>
            </div>
          )}
        </section>
      )}

      {comments.data !== undefined && (
        <section aria-label={copy.shown} className="grid gap-3">
          <ToggleGroup
            type="single"
            value={shown}
            aria-label={copy.shown}
            onValueChange={(value) => {
              if (value === "unresolved" || value === "resolved") {
                setShown(value);
              }
            }}
          >
            <ToggleGroupItem value="unresolved">{copy.unresolved}</ToggleGroupItem>
            <ToggleGroupItem value="resolved">{copy.resolved}</ToggleGroupItem>
          </ToggleGroup>
          {listed.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {shown === "resolved" ? copy.noneResolved : copy.none}
            </p>
          ) : (
            <ul className="grid gap-3 pb-4">
              {listed.map((thread) => (
                <ThreadItem
                  key={thread.id}
                  page={page}
                  thread={thread}
                  people={people}
                  mayComment={mayComment}
                  active={thread.id === active}
                  focusRequest={focusRequest}
                  onActive={onActive}
                  onReveal={onReveal}
                  onPosted={(result) => setUnseen(result.unnotified)}
                  onUnseen={unseenOne}
                />
              ))}
            </ul>
          )}
        </section>
      )}
    </aside>
  );
}

function isResolved(thread: Thread): boolean {
  return (thread.resolved_at ?? null) !== null;
}

/** Someone a comment names, as the workspace knows them now, or as the comment does. */
function someone(people: ReadonlyMap<string, Someone>, person: Person | null | undefined) {
  if (person === null || person === undefined) {
    return undefined;
  }
  return people.get(person.id) ?? person;
}

/** How someone is named, or `otherwise` when they are not known. */
function someoneNamed(person: Someone | undefined, otherwise: string): string {
  return person === undefined ? otherwise : attribution(person);
}

type ThreadItemProps = {
  page: Page;
  thread: Thread;
  people: ReadonlyMap<string, Someone>;
  mayComment: boolean;
  active: boolean;
  focusRequest: { threadId: string } | null;
  onActive: (threadId: string | null) => void;
  onReveal: (thread: Thread) => void;
  onPosted: (result: PostedComment) => void;
  onUnseen: (person: Person) => void;
};

function ThreadItem({
  page,
  thread,
  people,
  mayComment,
  active,
  focusRequest,
  onActive,
  onReveal,
  onPosted,
  onUnseen,
}: ThreadItemProps) {
  const workspace = useWorkspace();
  const reply = useReply(workspace.id, page.id);
  const resolve = useResolveThread(workspace.id, page.id);
  const [replying, setReplying] = useState(false);
  const item = useRef<HTMLElement>(null);
  const resolved = isResolved(thread);
  const anchor = thread.anchor ?? null;
  const first = thread.comments[0];
  const starter = someone(people, first?.author);
  const resolvedBy = thread.resolved_by ?? null;
  const resolver =
    resolvedBy === null ? copy.gone : someoneNamed(people.get(resolvedBy), copy.someone);

  useEffect(() => {
    if (active) {
      item.current?.scrollIntoView({ block: "nearest" });
    }
  }, [active]);
  useEffect(() => {
    if (focusRequest?.threadId === thread.id) {
      item.current?.focus();
    }
  }, [focusRequest, thread.id]);

  return (
    <li>
      <article
        ref={item}
        tabIndex={-1}
        aria-label={copy.thread(someoneNamed(starter, copy.gone))}
        {...(active ? { "data-active": "" } : {})}
        className="grid gap-3 rounded-lg border border-border bg-card p-3 outline-none focus-visible:ring-2 focus-visible:ring-ring data-active:border-primary"
      >
        {anchor !== null && (
          <div className="grid gap-1">
            <blockquote className="line-clamp-3 border-l-2 border-primary pl-2 text-sm text-muted-foreground">
              {anchor.quote}
            </blockquote>
            {anchor.detached ? (
              <p className="text-xs text-muted-foreground">{copy.detached}</p>
            ) : (
              <Button
                variant="link"
                size="sm"
                className="justify-self-start"
                onClick={() => {
                  onActive(thread.id);
                  onReveal(thread);
                }}
              >
                {copy.showInPage}
              </Button>
            )}
          </div>
        )}
        <ol className="grid gap-3">
          {thread.comments.map((comment) => (
            <CommentItem key={comment.id} page={page} comment={comment} people={people} />
          ))}
        </ol>
        {resolved && (
          <p className="text-xs text-muted-foreground">
            {copy.resolvedBy(resolver, formatDateTime(thread.resolved_at ?? thread.created_at))}
          </p>
        )}
        {(reply.isError || resolve.isError) && <ErrorNotice error={reply.error ?? resolve.error} />}
        {mayComment && replying && (
          <CommentComposer
            workspaceId={workspace.id}
            pageId={page.id}
            label={copy.replyLabel}
            submitLabel={copy.reply}
            onSubmit={async (markdown) => {
              onPosted(await reply.mutateAsync({ threadId: thread.id, markdown }));
              setReplying(false);
            }}
            onCancel={() => setReplying(false)}
            onUnseen={onUnseen}
          />
        )}
        {mayComment && !replying && (
          <div className="flex flex-wrap gap-1">
            {!resolved && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  onActive(thread.id);
                  setReplying(true);
                }}
              >
                {copy.reply}
              </Button>
            )}
            <Button
              variant="ghost"
              size="sm"
              disabled={resolve.isPending}
              onClick={() => resolve.mutate({ threadId: thread.id, resolved: !resolved })}
            >
              {resolved ? <RotateCcw aria-hidden /> : <Check aria-hidden />}
              {resolved ? copy.reopen : copy.resolve}
            </Button>
          </div>
        )}
      </article>
    </li>
  );
}

function CommentItem({
  page,
  comment,
  people,
}: {
  page: Page;
  comment: Comment;
  people: ReadonlyMap<string, Someone>;
}) {
  const workspace = useWorkspace();
  const me = useMe();
  const edit = useEditComment(workspace.id, page.id);
  const remove = useDeleteComment(workspace.id, page.id);
  const [editing, setEditing] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const author = someone(people, comment.author);
  const mine = comment.author?.id === me.id;
  const edited = (comment.edited_at ?? null) !== null;
  const when = formatDateTime(comment.created_at);
  const who = someoneNamed(author, copy.gone);

  return (
    <li className="grid gap-1">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <p className="flex min-w-0 flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
          <PersonLabel person={author} className="text-foreground" />
          <time dateTime={comment.created_at}>{when}</time>
          {edited && <span>({copy.edited})</span>}
        </p>
        {mine && !editing && (
          <div className="flex shrink-0">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={copy.actionLabel(copy.edit, who, when)}
              onClick={() => setEditing(true)}
            >
              <Pencil aria-hidden />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={copy.actionLabel(copy.delete, who, when)}
              disabled={remove.isPending}
              onClick={() => setConfirming(true)}
            >
              <Trash2 aria-hidden />
            </Button>
          </div>
        )}
      </div>
      {editing ? (
        <>
          {edit.isError && <ErrorNotice error={edit.error} />}
          <CommentComposer
            workspaceId={workspace.id}
            pageId={page.id}
            label={copy.editLabel}
            submitLabel={copy.save}
            initial={comment.markdown}
            onSubmit={async (markdown) => {
              await edit.mutateAsync({ commentId: comment.id, markdown });
              setEditing(false);
            }}
            onCancel={() => setEditing(false)}
          />
        </>
      ) : (
        <CommentContent runs={runsOf(comment)} slug={workspace.slug} people={people} />
      )}
      {remove.isError && <ErrorNotice error={remove.error} />}
      <ConfirmDialog
        open={confirming}
        title={copy.deleteTitle}
        description={copy.deleteLede}
        confirm={copy.delete}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          remove.mutate(comment.id);
        }}
      />
    </li>
  );
}
