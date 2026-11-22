import { useQuery } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { useState, type FormEvent } from "react";
import { hasCode } from "@/api/errors";
import {
  inviteLinksQuery,
  useCreateInviteLink,
  useInviteLinkToken,
  useRevokeInviteLink,
  type CreatedInviteLink,
  type InviteLink,
} from "@/api/workspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { CopyField } from "@/components/feedback/copy-field";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatDateTime } from "@/lib/format";
import { workspaceInviteLink } from "@/lib/links";
import { strings } from "@/lib/strings";
import { dayProblem, endOfDay, inAYear, inviteUsesProblem, today } from "@/lib/validation";
import { LINK_ROLES } from "@/lib/workspace-rules";

const copy = strings.workspaces.members;

/** Making invite links, showing a new one's address once, and every link with its uses. */
export function InviteLinks() {
  const workspace = useWorkspace();
  const links = useQuery(inviteLinksQuery(workspace.id));
  const [created, setCreated] = useState<CreatedInviteLink>();

  return (
    <PageSection title={copy.links} hint={copy.linksHint}>
      {created ? (
        <CreatedLinkNotice created={created} onDone={() => setCreated(undefined)} />
      ) : (
        <MakeLinkForm onCreated={setCreated} />
      )}
      {links.isPending && <Loading />}
      {links.isError && (
        <ErrorNotice
          error={links.error}
          retrying={links.isFetching}
          onRetry={() => void links.refetch()}
        />
      )}
      {links.data?.length === 0 && <p className="text-sm text-muted-foreground">{copy.noLinks}</p>}
      {links.data !== undefined && links.data.length > 0 && (
        <ul className="grid gap-3">
          {links.data.map((link) => (
            <LinkCard
              key={link.id}
              link={link}
              onRevoked={() => {
                if (created?.link.id === link.id) {
                  setCreated(undefined);
                }
              }}
            />
          ))}
        </ul>
      )}
    </PageSection>
  );
}

type Problems = { max_uses?: string | undefined; expires?: string | undefined };

function MakeLinkForm({ onCreated }: { onCreated: (created: CreatedInviteLink) => void }) {
  const workspace = useWorkspace();
  const create = useCreateInviteLink(workspace.id);
  const [role, setRole] = useState<(typeof LINK_ROLES)[number]>("member");
  const [uses, setUses] = useState("10");
  const [expires, setExpires] = useState("");
  const [problems, setProblems] = useState<Problems>({});
  const [bounds] = useState(() => ({ first: today(), last: inAYear() }));

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    create.reset();
    const found: Problems = {
      max_uses: inviteUsesProblem(uses),
      expires: dayProblem(expires, bounds.last),
    };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    create.mutate(
      {
        role,
        max_uses: Number(uses),
        ...(expires === "" ? {} : { expires_at: endOfDay(expires) }),
      },
      {
        onSuccess: (made) => {
          setUses("10");
          setExpires("");
          onCreated(made);
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.links}
      className="grid gap-4 rounded-lg border border-border bg-card p-4 sm:grid-cols-3"
    >
      {create.isError && (
        <div className="sm:col-span-3">
          {hasCode(create.error, "member_limit") ? (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">{copy.memberLimit}</p>
            </Alert>
          ) : (
            <ErrorNotice error={create.error} />
          )}
        </div>
      )}
      <Field label={copy.linkRole}>
        {(control) => (
          <NativeSelect
            {...control}
            name="role"
            value={role}
            onChange={(event) => setRole(event.target.value === "guest" ? "guest" : "member")}
          >
            {LINK_ROLES.map((option) => (
              <option key={option} value={option}>
                {strings.roles[option]}
              </option>
            ))}
          </NativeSelect>
        )}
      </Field>
      <Field label={copy.linkUses} hint={copy.linkUsesHint} error={problems.max_uses}>
        {(control) => (
          <Input
            {...control}
            name="max_uses"
            type="number"
            inputMode="numeric"
            min={1}
            max={1000}
            value={uses}
            onChange={(event) => setUses(event.target.value)}
          />
        )}
      </Field>
      <Field label={copy.linkExpires} hint={copy.linkExpiresHint} error={problems.expires}>
        {(control) => (
          <Input
            {...control}
            name="expires"
            type="date"
            min={bounds.first}
            max={bounds.last}
            value={expires}
            onChange={(event) => setExpires(event.target.value)}
          />
        )}
      </Field>
      <div className="sm:col-span-3">
        <Button type="submit" variant="outline" disabled={create.isPending}>
          <Plus aria-hidden />
          {create.isPending ? copy.linkPending : copy.linkSubmit}
        </Button>
      </div>
    </form>
  );
}

/** A new link's address, shown now; later only from its sealed copy, when the server keeps one. */
function CreatedLinkNotice({
  created,
  onDone,
}: {
  created: CreatedInviteLink;
  onDone: () => void;
}) {
  const { link, token } = created;
  return (
    <Alert live="polite">
      <AlertTitle>{copy.linkMade}</AlertTitle>
      <AlertDescription>
        <p>
          {copy.linkAdmits(
            link.max_uses,
            strings.roleNames[link.role],
            formatDateTime(link.expires_at),
          )}
        </p>
        <CopyField label={copy.link} value={workspaceInviteLink(token)} />
        {!link.token_kept && <p>{copy.linkShownOnce}</p>}
        <div>
          <Button variant="outline" size="sm" onClick={onDone}>
            {copy.done}
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  );
}

/**
 * A link with its uses. Its address, once shown, stays only while the link is
 * open: a revoked, used up or expired link admits nobody, so there is nothing
 * left to share.
 */
function LinkCard({ link, onRevoked }: { link: InviteLink; onRevoked: () => void }) {
  const workspace = useWorkspace();
  const revoke = useRevokeInviteLink(workspace.id);
  const reveal = useInviteLinkToken(workspace.id);
  const [confirming, setConfirming] = useState(false);
  const made = formatDateTime(link.created_at);
  const open = link.status === "open";

  return (
    <ItemCard>
      <div className="grid min-w-0 gap-2">
        <ChipList>
          <Chip tone={open ? "strong" : "default"}>{copy.statuses[link.status]}</Chip>
          <Chip>{copy.as(strings.roleNames[link.role])}</Chip>
        </ChipList>
        <p className="text-sm">{copy.linkUsed(link.uses, link.max_uses)}</p>
        <p className="text-sm text-muted-foreground">
          {copy.madeAt(made)} · {copy.endsAt(formatDateTime(link.expires_at))}
        </p>
        {open && !link.token_kept && (
          <p className="text-xs text-muted-foreground">{copy.notKept}</p>
        )}
        {open && reveal.data !== undefined && (
          <CopyField label={copy.link} value={workspaceInviteLink(reveal.data)} />
        )}
        {reveal.isError && <ErrorNotice error={reveal.error} />}
        {revoke.isError && <ErrorNotice error={revoke.error} />}
      </div>
      {open && (
        <ItemActions>
          {link.token_kept && reveal.data === undefined && (
            <Button
              variant="outline"
              size="sm"
              disabled={reveal.isPending}
              aria-label={copy.showLinkLabel(made)}
              onClick={() => reveal.mutate(link.id)}
            >
              {copy.showLink}
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={revoke.isPending}
            aria-label={copy.revokeLinkLabel(made)}
            onClick={() => setConfirming(true)}
          >
            {copy.revokeLink}
          </Button>
        </ItemActions>
      )}
      <ConfirmDialog
        open={confirming}
        title={copy.revokeLinkTitle}
        description={copy.revokeLinkLede}
        confirm={copy.revokeLink}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          revoke.mutate(link.id, {
            onSuccess: () => {
              reveal.reset();
              onRevoked();
            },
          });
        }}
      />
    </ItemCard>
  );
}
