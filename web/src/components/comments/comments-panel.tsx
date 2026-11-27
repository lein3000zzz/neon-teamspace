import { useInfiniteQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Check, MessageSquarePlus, Pencil, RotateCcw, Trash2, X } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import {
  isResolved,
  runsOf,
  threadCommentsQuery,
  threadsQuery,
  useDeleteComment,
  useDeleteThread,
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
  /** A thread to move focus to, as when the keyboard or a link opened it; each request a new object. */
  focusRequest: { threadId: string } | null;
  onActive: (threadId: string | null) => void;
  /** The thread a link names, shown in its part of the list though that part is not read yet. */
  linked?: Thread | undefined;
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
  linked,
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
  const [shown, setShown] = useState<Shown>("unresolved");
  const open = useInfiniteQuery(threadsQuery(workspace.id, page.id, false));
  const settled = useInfiniteQuery({
    ...threadsQuery(workspace.id, page.id, true),
    enabled: shown === "resolved",
  });
  const start = useStartThread(workspace.id, page.id);
  const follow = useFollowPage(workspace.id, page.id);
  const [unseen, setUnseen] = useState<readonly Person[]>([]);
  const [noSelection, setNoSelection] = useState(false);
  const openThreads = useMemo(
    () => open.data?.pages.flatMap((part) => part.threads) ?? [],
    [open.data],
  );
  const settledThreads = useMemo(
    () => settled.data?.pages.flatMap((part) => part.threads) ?? [],
    [settled.data],
  );
  const activeThread =
    openThreads.find((thread) => thread.id === active) ??
    settledThreads.find((thread) => thread.id === active) ??
    (linked?.id === active ? linked : undefined);
  // The thread opened shows in its part of the list, resolved or not.
  const [shownFor, setShownFor] = useState<string | null>(null);
  if (activeThread !== undefined && shownFor !== activeThread.id) {
    setShownFor(activeThread.id);
    setShown(isResolved(activeThread) ? "resolved" : "unresolved");
  }
  const comments = shown === "resolved" ? settled : open;
  const listed = withActive(
    shown === "resolved" ? settledThreads : openThreads,
    activeThread !== undefined && isResolved(activeThread) === (shown === "resolved")
      ? activeThread
      : undefined,
  );
  const flags = open.data?.pages[0];
  const mayComment = flags?.may_comment ?? false;
  const mayModerate = flags?.may_moderate ?? false;

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
      // The whole width of a phone, which a 24rem panel would leave a strip of; beside the
      // page from sm up.
      className="fixed inset-y-0 right-0 z-30 flex w-full flex-col gap-4 overflow-y-auto overscroll-contain border-border bg-raised safe-top-3 safe-right-3 safe-bottom-4 pl-3 sm:w-96 sm:border-l"
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
          {flags !== undefined && (
            <Button
              variant="outline"
              size="sm"
              aria-pressed={flags.following}
              title={copy.followHint}
              disabled={follow.isPending}
              onClick={() => follow.mutate(!flags.following)}
            >
              {flags.following && <Check aria-hidden />}
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

      {open.isPending && <Loading label={copy.loading} />}
      {open.isError && !open.isFetchNextPageError && (
        <ErrorNotice
          error={open.error}
          retrying={open.isFetching}
          onRetry={() => void open.refetch()}
        />
      )}

      {flags !== undefined && !mayComment && (
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
              {/* The shortcut is for a keyboard, which a touch screen as the main pointer lacks. */}
              <p
                className={cn(
                  "text-xs text-muted-foreground",
                  !noSelection && "pointer-coarse:hidden",
                )}
                role={noSelection ? "status" : undefined}
              >
                {noSelection ? copy.selectFirst : copy.shortcut}
              </p>
            </div>
          )}
        </section>
      )}

      {flags !== undefined && (
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
          {shown === "resolved" && settled.isError && !settled.isFetchNextPageError && (
            <ErrorNotice
              error={settled.error}
              retrying={settled.isFetching}
              onRetry={() => void settled.refetch()}
            />
          )}
          {comments.isPending ? (
            <Loading label={copy.loading} />
          ) : listed.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {shown === "resolved" ? copy.noneResolved : copy.none}
            </p>
          ) : (
            <ul className="grid gap-3">
              {listed.map((thread) => (
                <ThreadItem
                  key={thread.id}
                  page={page}
                  thread={thread}
                  people={people}
                  mayComment={mayComment}
                  mayModerate={mayModerate}
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
          {comments.isFetchNextPageError && <ErrorNotice error={comments.error} />}
          {comments.hasNextPage && (
            <Button
              variant="ghost"
              size="sm"
              className="justify-self-start"
              disabled={comments.isFetchingNextPage}
              onClick={() => void comments.fetchNextPage()}
            >
              {comments.isFetchingNextPage ? copy.loadingMore : copy.moreThreads}
            </Button>
          )}
          <div className="pb-4" />
        </section>
      )}
    </aside>
  );
}

/** A list of threads with the one open in the panel, in its place, should its part not be read yet. */
function withActive(threads: readonly Thread[], active: Thread | undefined): readonly Thread[] {
  if (active === undefined || threads.some((thread) => thread.id === active.id)) {
    return threads;
  }
  return [...threads, active].toSorted((a, b) => (a.id < b.id ? -1 : 1));
}

/**
 * A thread's comments as far as they are read: its first, those read after
 * it, then, after a gap of `hidden` others, its latest.
 */
function threadComments(thread: Thread, read: readonly Comment[]) {
  const [first, ...latest] = thread.comments;
  const head = first === undefined ? [] : [first];
  for (const comment of read) {
    if (!head.some((each) => each.id === comment.id)) {
      head.push(comment);
    }
  }
  const tail = latest.filter((comment) => !head.some((each) => each.id === comment.id));
  const hidden = Math.max(0, thread.comment_count - head.length - tail.length);
  return { head, tail, hidden };
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
  /** The user edits the page: they delete anyone's comment, and the thread. */
  mayModerate: boolean;
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
  mayModerate,
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
  const removeThread = useDeleteThread(workspace.id, page.id);
  const [replying, setReplying] = useState(false);
  const [confirming, setConfirming] = useState(false);
  // The replies between the first and the latest, read once asked for.
  const [earlier, setEarlier] = useState(false);
  const middle = useInfiniteQuery({
    ...threadCommentsQuery(workspace.id, page.id, thread),
    enabled: earlier,
  });
  const read = useMemo(
    () => middle.data?.pages.flatMap((part) => part.comments) ?? [],
    [middle.data],
  );
  const { head, tail, hidden } = threadComments(thread, read);
  const item = useRef<HTMLElement>(null);
  const quoteId = useId();
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
        // Read out with its name when focus comes to it, as a link to it moves focus here.
        {...(anchor === null ? {} : { "aria-describedby": quoteId })}
        {...(active ? { "data-active": "" } : {})}
        className="grid gap-3 rounded-lg border border-border bg-card p-3 outline-none focus-visible:ring-2 focus-visible:ring-ring data-active:border-primary"
      >
        {anchor !== null && (
          <div className="grid gap-1">
            <blockquote
              id={quoteId}
              className="line-clamp-3 border-l-2 border-primary pl-2 text-sm text-muted-foreground"
            >
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
          {[...head, ...tail].map((comment, at) => (
            <CommentItem
              key={comment.id}
              page={page}
              comment={comment}
              people={people}
              mayModerate={mayModerate}
              onPosted={onPosted}
              onUnseen={onUnseen}
              before={
                at === head.length && hidden > 0 ? (
                  <Button
                    variant="link"
                    size="sm"
                    className="justify-self-start"
                    disabled={middle.isFetching}
                    onClick={() => (earlier ? void middle.fetchNextPage() : setEarlier(true))}
                  >
                    {middle.isFetching ? copy.loadingMore : copy.earlier(hidden)}
                  </Button>
                ) : null
              }
            />
          ))}
        </ol>
        {middle.isError && <ErrorNotice error={middle.error} />}
        {resolved && (
          <p className="text-xs text-muted-foreground">
            {copy.resolvedBy(resolver, formatDateTime(thread.resolved_at ?? thread.created_at))}
          </p>
        )}
        {(reply.isError || resolve.isError || removeThread.isError) && (
          <ErrorNotice error={reply.error ?? resolve.error ?? removeThread.error} />
        )}
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
            {mayModerate && (
              <Button
                variant="ghost"
                size="sm"
                disabled={removeThread.isPending}
                onClick={() => setConfirming(true)}
              >
                <Trash2 aria-hidden />
                {copy.deleteThread}
              </Button>
            )}
          </div>
        )}
        <ConfirmDialog
          open={confirming}
          title={copy.deleteThreadTitle}
          description={copy.deleteThreadLede}
          confirm={copy.delete}
          onCancel={() => setConfirming(false)}
          onConfirm={() => {
            setConfirming(false);
            removeThread.mutate(thread.id);
          }}
        />
      </article>
    </li>
  );
}

type CommentItemProps = {
  page: Page;
  comment: Comment;
  people: ReadonlyMap<string, Someone>;
  /** The user edits the page: they delete this comment, whoever wrote it. */
  mayModerate: boolean;
  onPosted: (result: PostedComment) => void;
  onUnseen: (person: Person) => void;
  /** What shows before it, as the replies left out before the latest. */
  before: ReactNode;
};

function CommentItem({
  page,
  comment,
  people,
  mayModerate,
  onPosted,
  onUnseen,
  before,
}: CommentItemProps) {
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
      {before}
      <div className="flex min-w-0 items-center justify-between gap-2">
        <p className="flex min-w-0 flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
          <PersonLabel person={author} className="text-foreground" />
          <time dateTime={comment.created_at}>{when}</time>
          {edited && <span>({copy.edited})</span>}
        </p>
        {(mine || mayModerate) && !editing && (
          <div className="flex shrink-0">
            {mine && (
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={copy.actionLabel(copy.edit, who, when)}
                onClick={() => setEditing(true)}
              >
                <Pencil aria-hidden />
              </Button>
            )}
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
              onPosted(await edit.mutateAsync({ commentId: comment.id, markdown }));
              setEditing(false);
            }}
            onCancel={() => setEditing(false)}
            onUnseen={onUnseen}
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
          remove.mutate({ commentId: comment.id, threadId: comment.thread_id });
        }}
      />
    </li>
  );
}
