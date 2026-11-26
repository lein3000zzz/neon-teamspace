import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { connectedAppsQuery, useDisconnectApp, type ConnectedApp } from "@/api/oauth";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Chip, ChipList } from "@/components/nodes/chip";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { PersonLabel } from "@/components/people/person-label";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { Button } from "@/components/ui/button";
import { formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";

const copy = strings.account.apps;

/**
 * The apps the person connected over OAuth, each in one workspace as them or
 * as one of their agents, with what it may do and when it was last used, and
 * disconnecting each.
 */
export function ConnectedAppsPage() {
  const apps = useQuery(connectedAppsQuery);
  const [done, setDone] = useState<string>();

  return (
    <PageSection title={copy.title} hint={copy.hint}>
      <output className="block text-sm text-muted-foreground empty:hidden">{done ?? ""}</output>
      {apps.isPending && <Loading />}
      {apps.isError && (
        <ErrorNotice
          error={apps.error}
          retrying={apps.isFetching}
          onRetry={() => void apps.refetch()}
        />
      )}
      {apps.data?.length === 0 && <p className="text-sm text-muted-foreground">{copy.none}</p>}
      {apps.data && apps.data.length > 0 && (
        <ul className="grid gap-3">
          {apps.data.map((app) => (
            <AppCard key={app.id} app={app} onDone={setDone} />
          ))}
        </ul>
      )}
    </PageSection>
  );
}

function AppCard({ app, onDone }: { app: ConnectedApp; onDone: (message: string) => void }) {
  const disconnect = useDisconnectApp();
  const [confirming, setConfirming] = useState(false);
  const name = app.client.name;

  return (
    <ItemCard aria-label={name}>
      <div className="grid gap-2">
        <p className="font-medium">{name}</p>
        <ChipList aria-label={copy.scopes}>
          {app.scopes.map((scope) => (
            <Chip key={scope}>{strings.consent.scopeNames[scope]}</Chip>
          ))}
        </ChipList>
        <KeyValueList className="text-sm">
          <KeyValueRow label={copy.workspace}>{app.workspace.name}</KeyValueRow>
          <KeyValueRow label={copy.actsAs}>
            {app.identity.kind === "human" ? copy.you : <PersonLabel person={app.identity} />}
          </KeyValueRow>
          <KeyValueRow label={copy.connected}>{formatDateTime(app.created_at)}</KeyValueRow>
          <KeyValueRow label={copy.lastUsed}>
            {app.last_used_at ? formatDateTime(app.last_used_at) : copy.never}
          </KeyValueRow>
        </KeyValueList>
        {disconnect.isError && <ErrorNotice error={disconnect.error} />}
      </div>
      <ItemActions>
        <Button
          variant="outline"
          size="sm"
          disabled={disconnect.isPending}
          aria-label={copy.disconnectLabel(name, app.workspace.name)}
          onClick={() => setConfirming(true)}
        >
          {copy.disconnect}
        </Button>
      </ItemActions>
      <ConfirmDialog
        open={confirming}
        title={copy.disconnectTitle(name)}
        description={copy.disconnectLede}
        confirm={copy.disconnect}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          disconnect.mutate(app.id, { onSuccess: () => onDone(copy.disconnected(name)) });
        }}
      />
    </ItemCard>
  );
}
