import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import {
  joinRequestsQuery,
  useApproveJoin,
  useDenyJoin,
  type CreatedInvite,
  type JoinRequest,
} from "@/api/admin";
import { authConfigQuery } from "@/api/auth";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Chip, ChipList } from "@/components/nodes/chip";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { Button } from "@/components/ui/button";
import { formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";
import { CreatedInviteNotice } from "./created-invite";

const copy = strings.admin.requests;

/**
 * Requests to join that their address confirmed, waiting first, then those
 * decided in the last 30 days. Approving mails the address an invite bound
 * to it; denying mails nothing, or a note that they may ask again later.
 */
export function RequestsPage() {
  const config = useQuery(authConfigQuery);
  const requests = useQuery(joinRequestsQuery);
  const [approved, setApproved] = useState<CreatedInvite>();

  return (
    <PageSection title={strings.admin.tabs.requests}>
      {config.data && !config.data.requests && (
        <p className="text-sm text-muted-foreground">
          {copy.off}{" "}
          <Link to="/admin/settings" className="text-foreground underline underline-offset-4">
            {copy.changeSetting}
          </Link>
          .
        </p>
      )}
      {approved && <CreatedInviteNotice created={approved} onDone={() => setApproved(undefined)} />}

      {requests.isPending && <Loading />}
      {requests.isError && (
        <ErrorNotice
          error={requests.error}
          retrying={requests.isFetching}
          onRetry={() => void requests.refetch()}
        />
      )}
      {requests.data?.length === 0 && <p className="text-sm text-muted-foreground">{copy.none}</p>}
      {requests.data && requests.data.length > 0 && (
        <ul className="grid gap-3">
          {requests.data.map((request) => (
            <RequestCard key={request.id} request={request} onApproved={setApproved} />
          ))}
        </ul>
      )}
    </PageSection>
  );
}

function RequestCard({
  request,
  onApproved,
}: {
  request: JoinRequest;
  onApproved: (invite: CreatedInvite) => void;
}) {
  const approve = useApproveJoin();
  const deny = useDenyJoin();
  const [denying, setDenying] = useState(false);
  const [denied, setDenied] = useState(false);
  const waiting = request.status === "pending";
  const busy = approve.isPending || deny.isPending;

  return (
    <ItemCard>
      <div className="grid min-w-0 gap-2">
        <p className="font-medium break-all">{request.email}</p>
        <ChipList>
          <Chip tone={waiting ? "strong" : "default"}>{copy.statuses[request.status]}</Chip>
          <Chip>{copy.asked(formatDateTime(request.confirmed_at ?? request.created_at))}</Chip>
          {request.decided_at && <Chip>{copy.decided(formatDateTime(request.decided_at))}</Chip>}
        </ChipList>
        <p className="text-sm whitespace-pre-wrap text-muted-foreground">
          {request.note === "" ? copy.noNote : request.note}
        </p>
        {approve.isError && <ErrorNotice error={approve.error} />}
        {deny.isError && <ErrorNotice error={deny.error} />}
        <output className="block text-sm text-muted-foreground empty:hidden">
          {denied ? copy.denied(request.email) : ""}
        </output>
      </div>
      {waiting && (
        <ItemActions>
          <Button
            size="sm"
            disabled={busy}
            aria-label={copy.actionLabel(copy.approve, request.email)}
            onClick={() => approve.mutate(request.id, { onSuccess: onApproved })}
          >
            {copy.approve}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            aria-label={copy.actionLabel(copy.deny, request.email)}
            onClick={() => setDenying(true)}
          >
            {copy.deny}
          </Button>
        </ItemActions>
      )}
      <ConfirmDialog
        open={denying}
        title={copy.denyTitle(request.email)}
        description={copy.denyLede}
        confirm={copy.deny}
        onCancel={() => setDenying(false)}
        onConfirm={() => {
          setDenying(false);
          deny.mutate(request.id, { onSuccess: () => setDenied(true) });
        }}
      />
    </ItemCard>
  );
}
