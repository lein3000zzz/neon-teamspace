import { useQuery } from "@tanstack/react-query";
import { LogOut } from "lucide-react";
import {
  sessionsQuery,
  useRevokeOtherSessions,
  useRevokeSession,
  type SessionInfo,
} from "@/api/account";
import { useLogout } from "@/api/auth";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Chip } from "@/components/nodes/chip";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { Button } from "@/components/ui/button";
import { describeUserAgent, formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";

const copy = strings.account.sessions;

/**
 * Every browser signed in to the account, newest first. Any but this one can
 * be signed out from here; this one signs out as the sidebar's button does.
 */
export function SessionsPage() {
  const sessions = useQuery(sessionsQuery);
  const others = useRevokeOtherSessions();
  const hasOthers = sessions.data?.some((session) => !session.current) ?? false;

  return (
    <PageSection
      title={copy.title}
      hint={copy.hint}
      actions={
        <Button
          variant="outline"
          disabled={!hasOthers || others.isPending}
          onClick={() => others.mutate()}
        >
          {others.isPending ? copy.othersPending : copy.others}
        </Button>
      }
    >
      <output className="block text-sm text-muted-foreground empty:hidden">
        {others.isSuccess ? copy.othersDone(others.data) : ""}
      </output>
      {others.isError && <ErrorNotice error={others.error} />}

      {sessions.isPending && <Loading />}
      {sessions.isError && (
        <ErrorNotice
          error={sessions.error}
          retrying={sessions.isFetching}
          onRetry={() => void sessions.refetch()}
        />
      )}
      {sessions.data && (
        <ul className="grid gap-3">
          {sessions.data.map((session) => (
            <SessionCard key={session.id} session={session} />
          ))}
        </ul>
      )}
    </PageSection>
  );
}

function SessionCard({ session }: { session: SessionInfo }) {
  const revoke = useRevokeSession();
  const logout = useLogout();
  const browser = describeUserAgent(session.user_agent) ?? copy.unknownBrowser;
  const started = formatDateTime(session.created_at);
  const action = session.current ? logout : revoke;

  return (
    <ItemCard>
      <div className="grid gap-2">
        <p className="flex flex-wrap items-center gap-2 font-medium">
          {browser}
          {session.current && <Chip tone="strong">{copy.current}</Chip>}
        </p>
        <KeyValueList className="text-sm">
          <KeyValueRow label={copy.address}>
            <span className="font-mono">{session.ip}</span>
          </KeyValueRow>
          <KeyValueRow label={copy.lastSeen}>{formatDateTime(session.last_seen_at)}</KeyValueRow>
          <KeyValueRow label={copy.started}>{started}</KeyValueRow>
          {/* The idle end, which the server never lets pass the absolute one. */}
          <KeyValueRow label={copy.ends}>{formatDateTime(session.expires_at)}</KeyValueRow>
        </KeyValueList>
        {action.isError && <ErrorNotice error={action.error} />}
      </div>
      <ItemActions>
        <Button
          variant="outline"
          size="sm"
          disabled={action.isPending}
          aria-label={copy.signOutLabel(session.current ? copy.current : browser, started)}
          onClick={() => (session.current ? logout.mutate() : revoke.mutate(session.id))}
        >
          <LogOut aria-hidden />
          {copy.signOut}
        </Button>
      </ItemActions>
    </ItemCard>
  );
}
