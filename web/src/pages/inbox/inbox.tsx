import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Archive, ArchiveRestore, Check, CheckCheck, Mail } from "lucide-react";
import { useState } from "react";
import {
  inboxQuery,
  notificationTarget,
  useMarkAllRead,
  useMarkNotification,
  type InboxFilter,
  type Notification,
} from "@/api/inbox";
import { myInvitationsQuery } from "@/api/workspaces";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { HueDot } from "@/components/nodes/hue-dot";
import { ItemActions, PageSection } from "@/components/settings/page-section";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { InvitationList } from "@/components/workspaces/invitation-list";
import { formatDateTime } from "@/lib/format";
import { titleOf } from "@/lib/page-tree";
import { attributionInSentence } from "@/lib/people";
import { strings } from "@/lib/strings";

const copy = strings.inbox;

const FILTERS: readonly InboxFilter[] = ["unread", "all", "archived"];

/**
 * The user's inbox, across every workspace: the invitations that wait for an
 * answer, then the notifications, unread ones by default, each a link to the
 * comment or block it is about that marks it read as it opens. The server
 * checks each against what the user may see now, so one about a page they
 * lost goes from the list. It follows the live feed the shell keeps open.
 */
export function InboxPage() {
  const [filter, setFilter] = useState<InboxFilter>("unread");
  const invitations = useQuery(myInvitationsQuery);
  // Shown once there were any, so what accepting the last one said stays on screen.
  const [hadInvitations, setHadInvitations] = useState(false);
  if (!hadInvitations && (invitations.data?.length ?? 0) > 0) {
    setHadInvitations(true);
  }
  const markAll = useMarkAllRead();

  return (
    <>
      <PageHeader eyebrow={copy.eyebrow} title={copy.title} lede={copy.lede} />
      <div className="grid gap-10">
        {hadInvitations && (
          <PageSection title={copy.invitations}>
            <InvitationList />
          </PageSection>
        )}
        <PageSection
          title={copy.notifications}
          actions={
            <Button
              variant="outline"
              size="sm"
              disabled={markAll.isPending}
              onClick={() => markAll.mutate()}
            >
              <CheckCheck aria-hidden />
              {copy.markAllRead}
            </Button>
          }
        >
          <ToggleGroup
            type="single"
            value={filter}
            aria-label={copy.filter}
            onValueChange={(value) => {
              if (value !== "") {
                setFilter(value as InboxFilter);
              }
            }}
          >
            {FILTERS.map((each) => (
              <ToggleGroupItem key={each} value={each}>
                {copy.filters[each]}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          {markAll.isError && <ErrorNotice error={markAll.error} />}
          <NotificationList key={filter} filter={filter} />
        </PageSection>
      </div>
    </>
  );
}

function NotificationList({ filter }: { filter: InboxFilter }) {
  const notifications = useInfiniteQuery(inboxQuery(filter));
  // Invitations show above, with the way to answer them.
  const list = (notifications.data?.pages.flatMap((part) => part.notifications) ?? []).filter(
    (notification) => notification.kind !== "invitation",
  );
  if (notifications.isPending) {
    return <Loading label={copy.loading} />;
  }
  if (notifications.isError) {
    return (
      <ErrorNotice
        error={notifications.error}
        retrying={notifications.isFetching}
        onRetry={() => void notifications.refetch()}
      />
    );
  }
  return (
    <>
      {list.length === 0 && !notifications.hasNextPage ? (
        <p className="text-sm text-muted-foreground">{copy.none[filter]}</p>
      ) : (
        <ul className="grid gap-2">
          {list.map((notification) => (
            <NotificationItem key={notification.id} notification={notification} />
          ))}
        </ul>
      )}
      {notifications.hasNextPage && (
        <Button
          variant="outline"
          className="justify-self-center"
          disabled={notifications.isFetchingNextPage}
          onClick={() => void notifications.fetchNextPage()}
        >
          {notifications.isFetchingNextPage ? copy.loadingMore : copy.more}
        </Button>
      )}
    </>
  );
}

/** What a notification tells, on one line, as the server's mail says it. */
function summaryOf(notification: Notification): string {
  const who =
    notification.actor === undefined || notification.actor === null
      ? copy.someone
      : attributionInSentence(notification.actor);
  const page =
    notification.page === undefined || notification.page === null
      ? strings.pages.untitled
      : titleOf(notification.page);
  const said = copy.summaries;
  switch (notification.kind) {
    case "mention":
      return (notification.comment_id ?? null) === null
        ? said.mention(who, page)
        : said.mentionInComment(who, page);
    case "reply":
      return said.reply(who, page);
    case "comment":
      return said.comment(who, page);
    case "assigned":
      return said.assigned(who, page);
    case "invitation":
      return said.invitation(who, notification.workspace.name);
  }
}

function NotificationItem({ notification }: { notification: Notification }) {
  const mark = useMarkNotification();
  const target = notificationTarget(notification);
  const summary = summaryOf(notification);
  const unread = (notification.read_at ?? null) === null;
  const archived = (notification.archived_at ?? null) !== null;
  const excerpt = notification.excerpt ?? null;
  const { workspace } = notification;
  const label = (action: string) => copy.actionLabel(action, summary);

  return (
    <li
      className="grid gap-3 rounded-lg border border-border bg-card p-3 data-unread:border-primary/50 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-start"
      {...(unread ? { "data-unread": "" } : {})}
    >
      <div className="grid min-w-0 gap-1">
        <p className="flex min-w-0 items-start gap-2 text-sm">
          {unread && (
            <span className="mt-1.5 size-2 shrink-0 rounded-full bg-primary">
              <span className="sr-only">{copy.unreadMark}</span>
            </span>
          )}
          {target === undefined ? (
            <span className="min-w-0 font-medium break-words">{summary}</span>
          ) : (
            <Link
              to="/w/$slug/p/$pageId"
              params={{ slug: target.slug, pageId: target.pageId }}
              search={target.search}
              onClick={() => {
                if (unread) {
                  mark.mutate({ id: notification.id, read: true });
                }
              }}
              className="min-w-0 font-medium break-words underline-offset-4 hover:underline"
            >
              {summary}
            </Link>
          )}
        </p>
        {excerpt !== null && excerpt !== "" && (
          <p className="line-clamp-3 text-sm break-words text-muted-foreground">{excerpt}</p>
        )}
        <p className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
          <span className="inline-flex items-center gap-1.5">
            <HueDot hue={workspace.hue} className="size-1.5" />
            {copy.inWorkspace(workspace.name)}
          </span>
          <span aria-hidden>·</span>
          <time dateTime={notification.created_at}>{formatDateTime(notification.created_at)}</time>
        </p>
        {mark.isError && <ErrorNotice error={mark.error} />}
      </div>
      <ItemActions>
        <Button
          variant="ghost"
          size="sm"
          disabled={mark.isPending}
          aria-label={label(unread ? copy.markRead : copy.markUnread)}
          onClick={() => mark.mutate({ id: notification.id, read: unread })}
        >
          {unread ? <Check aria-hidden /> : <Mail aria-hidden />}
          {unread ? copy.markRead : copy.markUnread}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={mark.isPending}
          aria-label={label(archived ? copy.unarchive : copy.archive)}
          onClick={() => mark.mutate({ id: notification.id, archived: !archived })}
        >
          {archived ? <ArchiveRestore aria-hidden /> : <Archive aria-hidden />}
          {archived ? copy.unarchive : copy.archive}
        </Button>
      </ItemActions>
    </li>
  );
}
