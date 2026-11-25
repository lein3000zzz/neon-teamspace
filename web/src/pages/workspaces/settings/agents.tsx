import { useQueries, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { KeyRound } from "lucide-react";
import { useId, useState, type FormEvent } from "react";
import type { ApiToken } from "@/api/account";
import {
  agentsQuery,
  agentTokensQuery,
  useCreateAgent,
  useCreateAgentToken,
  useDeleteAgent,
  useRevokeAgentToken,
  useTransferAgent,
  useUpdateAgent,
  type Agent,
  type CreatedApiToken,
} from "@/api/agents";
import { hasCode } from "@/api/errors";
import { teamspaceMembersQuery, teamspacesQuery, useAddTeamspaceMember } from "@/api/teamspaces";
import { membersQuery, workspaceQuery, type Workspace } from "@/api/workspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { CopyField } from "@/components/feedback/copy-field";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { CheckboxField, Field, focusFirstInvalid } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { HueDot } from "@/components/nodes/hue-dot";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { KindBadge } from "@/components/people/person-label";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { useMe } from "@/components/shell/signed-in";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { HueControl } from "@/components/workspaces/hue-control";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatDate, formatDateTime } from "@/lib/format";
import { actsForOwner } from "@/lib/people";
import { strings } from "@/lib/strings";
import { displayNameProblem, required, tokenNameProblem } from "@/lib/validation";
import { manages, managesAgent, mayCreateAgent } from "@/lib/workspace-rules";

const copy = strings.workspaces.agents;

/** What an agent's token may have. */
const AGENT_SCOPES = ["read", "write"] as const;
type AgentScope = (typeof AGENT_SCOPES)[number];

/**
 * The workspace's agents and integrations: making one, for whom the workspace
 * lets, and for each its owner and status, renaming and recolouring it for its
 * owner, handing it over for owners and admins, deleting it, its tokens, and
 * adding it to teamspaces.
 */
export function AgentsSettingsPage() {
  const workspace = useWorkspace();
  const details = useQuery(workspaceQuery(workspace.id));
  const [done, setDone] = useState<string>();

  if (details.isPending) {
    return <Loading />;
  }
  if (details.isError) {
    return (
      <ErrorNotice
        error={details.error}
        retrying={details.isFetching}
        onRetry={() => void details.refetch()}
      />
    );
  }
  const { agents } = details.data;
  return (
    <div className="grid gap-10">
      {!agents.available && (
        <Alert live="polite">
          <p className="text-foreground">{copy.offInstance}</p>
        </Alert>
      )}
      {agents.available && !agents.enabled && (
        <Alert live="polite">
          <p className="text-foreground">{copy.offWorkspace}</p>
        </Alert>
      )}
      <CreateSection workspace={details.data} onDone={setDone} />
      <PageSection title={copy.list} hint={copy.listHint}>
        <output className="block text-sm text-muted-foreground empty:hidden">{done ?? ""}</output>
        <AgentList onDone={setDone} />
      </PageSection>
    </div>
  );
}

function CreateSection({
  workspace,
  onDone,
}: {
  workspace: Workspace;
  onDone: (message: string) => void;
}) {
  const create = useCreateAgent(workspace.id);
  const [name, setName] = useState("");
  const [hue, setHue] = useState<number | null>(null);
  const [problem, setProblem] = useState<string>();

  if (!mayCreateAgent(workspace.role, workspace.agents)) {
    return workspace.agents.available && workspace.agents.enabled && workspace.role !== "guest" ? (
      <PageSection title={copy.create}>
        <p className="text-sm text-muted-foreground">{copy.createAdmins}</p>
      </PageSection>
    ) : null;
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    create.reset();
    const found = required(name) ?? displayNameProblem(name);
    setProblem(found);
    if (found !== undefined) {
      focusFirstInvalid(event.currentTarget, ["name"]);
      return;
    }
    create.mutate(
      { name: name.trim(), ...(hue === null ? {} : { hue_override: hue }) },
      {
        onSuccess: (agent) => {
          setName("");
          setHue(null);
          onDone(copy.created(agent.name));
        },
      },
    );
  };

  return (
    <PageSection title={copy.create} hint={copy.createHint}>
      <form
        noValidate
        onSubmit={submit}
        aria-label={copy.create}
        className="grid max-w-xl gap-4 rounded-lg border border-border bg-card p-4"
      >
        {create.isError &&
          (hasCode(create.error, "agent_limit") ? (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">{copy.agentLimit}</p>
            </Alert>
          ) : (
            <ErrorNotice error={create.error} />
          ))}
        <Field label={copy.name} error={problem}>
          {(control) => (
            <Input
              {...control}
              name="name"
              autoComplete="off"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          )}
        </Field>
        <HueControl name={name.trim()} value={hue} onChange={setHue} />
        <Button type="submit" className="justify-self-start" disabled={create.isPending}>
          {create.isPending ? copy.pending : copy.submit}
        </Button>
      </form>
    </PageSection>
  );
}

function AgentList({ onDone }: { onDone: (message: string) => void }) {
  const workspace = useWorkspace();
  const agents = useQuery(agentsQuery(workspace.id));
  if (agents.isPending) {
    return <Loading />;
  }
  if (agents.isError) {
    return (
      <ErrorNotice
        error={agents.error}
        retrying={agents.isFetching}
        onRetry={() => void agents.refetch()}
      />
    );
  }
  if (agents.data.length === 0) {
    return <p className="text-sm text-muted-foreground">{copy.none}</p>;
  }
  return (
    <ul className="grid gap-3">
      {agents.data.map((agent) => (
        <AgentCard key={agent.id} agent={agent} onDone={onDone} />
      ))}
    </ul>
  );
}

/** What is open on an agent's card: its edit form, or the handing over waiting to be confirmed. */
type Open = { kind: "edit" } | { kind: "delete" } | { kind: "transfer"; to: string; name: string };

function AgentCard({ agent, onDone }: { agent: Agent; onDone: (message: string) => void }) {
  const me = useMe();
  const workspace = useWorkspace();
  const remove = useDeleteAgent(workspace.id);
  const [open, setOpen] = useState<Open>();
  const own = agent.owner?.id === me.id;
  const live = agent.status !== "deleted";
  const managing = managesAgent(workspace.role, own);
  const status = agent.status;

  return (
    <ItemCard aria-label={agent.name}>
      <div className="grid min-w-0 gap-2">
        <p className="flex min-w-0 flex-wrap items-center gap-2 font-medium">
          <HueDot hue={agent.hue} />
          <span className="break-words">{agent.name}</span>
          <KindBadge kind={agent.kind} />
        </p>
        <ChipList>
          <Chip tone={status === "active" ? "strong" : "default"} title={copy.statusHints[status]}>
            {copy.statuses[status]}
          </Chip>
          <Chip>{agent.owner ? copy.owner(agent.owner.display_name) : copy.noOwner}</Chip>
          {own && <Chip>{copy.you}</Chip>}
          <Chip>{copy.made(formatDate(agent.created_at))}</Chip>
        </ChipList>
        {status !== "active" && (
          <p className="text-sm text-muted-foreground">{copy.statusHints[status]}</p>
        )}
        {remove.isError && <ErrorNotice error={remove.error} />}
        {open?.kind === "edit" && (
          <EditForm agent={agent} onClose={() => setOpen(undefined)} onDone={onDone} />
        )}
        {live && manages(workspace.role) && (
          <TransferForm
            agent={agent}
            onChoose={(to, name) => setOpen({ kind: "transfer", to, name })}
            pending={open?.kind === "transfer" ? open : undefined}
            onSettled={() => setOpen(undefined)}
            onDone={onDone}
          />
        )}
        {live && managing && <AddToTeamspace agent={agent} own={own} onDone={onDone} />}
        {agent.kind === "agent" && managing && <Tokens agent={agent} own={own} />}
      </div>
      {live && (own || managing) && (
        <ItemActions>
          <Button variant="outline" size="sm" asChild>
            <Link
              to="/w/$slug/settings/webhooks"
              params={{ slug: workspace.slug }}
              search={{ for: agent.id }}
              aria-label={copy.webhooksLabel(agent.name)}
            >
              {copy.webhooks}
            </Link>
          </Button>
          {own && open?.kind !== "edit" && (
            <Button
              variant="outline"
              size="sm"
              aria-label={copy.editLabel(agent.name)}
              onClick={() => setOpen({ kind: "edit" })}
            >
              {copy.edit}
            </Button>
          )}
          {managing && (
            <Button
              variant="outline"
              size="sm"
              disabled={remove.isPending}
              aria-label={copy.deleteLabel(agent.name)}
              onClick={() => setOpen({ kind: "delete" })}
            >
              {copy.delete}
            </Button>
          )}
        </ItemActions>
      )}
      <ConfirmDialog
        open={open?.kind === "delete"}
        title={copy.deleteTitle(agent.name)}
        description={copy.deleteLede}
        confirm={copy.delete}
        onCancel={() => setOpen(undefined)}
        onConfirm={() => {
          setOpen(undefined);
          remove.mutate(agent.id, { onSuccess: () => onDone(copy.deleted(agent.name)) });
        }}
      />
    </ItemCard>
  );
}

function EditForm({
  agent,
  onClose,
  onDone,
}: {
  agent: Agent;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const workspace = useWorkspace();
  const update = useUpdateAgent(workspace.id);
  const [name, setName] = useState(agent.name);
  const [hue, setHue] = useState<number | null>(agent.hue_override ?? null);
  const [problem, setProblem] = useState<string>();

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const found = required(name) ?? displayNameProblem(name);
    setProblem(found);
    if (found !== undefined) {
      focusFirstInvalid(event.currentTarget, ["name"]);
      return;
    }
    update.mutate(
      { agentId: agent.id, body: { name: name.trim(), hue_override: hue } },
      {
        onSuccess: (saved) => {
          onDone(copy.saved(saved.name));
          onClose();
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.editLabel(agent.name)}
      className="grid gap-4 rounded-md border border-border p-3"
    >
      {update.isError && <ErrorNotice error={update.error} />}
      <Field label={copy.name} error={problem}>
        {(control) => (
          <Input
            {...control}
            name="name"
            autoComplete="off"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        )}
      </Field>
      <HueControl id={agent.id} name={name} value={hue} onChange={setHue} />
      <div className="flex flex-wrap gap-2">
        <Button type="submit" size="sm" disabled={update.isPending}>
          {update.isPending ? copy.saving : copy.save}
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onClose}>
          {copy.cancel}
        </Button>
      </div>
    </form>
  );
}

type TransferFormProps = {
  agent: Agent;
  pending: { to: string; name: string } | undefined;
  onChoose: (to: string, name: string) => void;
  onSettled: () => void;
  onDone: (message: string) => void;
};

/** For owners and admins: another person of the workspace, not a guest, to hand it to. */
function TransferForm({ agent, pending, onChoose, onSettled, onDone }: TransferFormProps) {
  const workspace = useWorkspace();
  const members = useQuery(membersQuery(workspace.id));
  const transfer = useTransferAgent(workspace.id);
  const [to, setTo] = useState("");
  const selectId = useId();
  const people =
    members.data?.filter(
      (member) =>
        !actsForOwner(member.user) && member.role !== "guest" && member.user.id !== agent.owner?.id,
    ) ?? [];
  if (people.length === 0) {
    return null;
  }
  return (
    <div className="grid gap-2">
      {transfer.isError && <ErrorNotice error={transfer.error} />}
      <div className="flex flex-wrap items-end gap-2">
        <label htmlFor={selectId} className="sr-only">
          {copy.transferTo}
        </label>
        <span className="w-56">
          <NativeSelect
            id={selectId}
            aria-label={copy.transferLabel(agent.name)}
            value={to}
            onChange={(event) => setTo(event.target.value)}
          >
            <option value="">{copy.transferChoose}</option>
            {people.map((member) => (
              <option key={member.user.id} value={member.user.id}>
                {member.user.display_name}
              </option>
            ))}
          </NativeSelect>
        </span>
        <Button
          variant="outline"
          size="sm"
          disabled={to === "" || transfer.isPending}
          onClick={() => {
            const person = people.find((member) => member.user.id === to);
            if (person !== undefined) {
              onChoose(to, person.user.display_name);
            }
          }}
        >
          {copy.transfer}
        </Button>
      </div>
      <ConfirmDialog
        open={pending !== undefined}
        title={pending === undefined ? "" : copy.transferTitle(agent.name, pending.name)}
        description={copy.transferLede}
        confirm={copy.transfer}
        onCancel={onSettled}
        onConfirm={() => {
          if (pending === undefined) {
            return;
          }
          onSettled();
          transfer.mutate(
            { agentId: agent.id, ownerId: pending.to },
            {
              onSuccess: () => {
                setTo("");
                onDone(copy.transferred(agent.name, pending.name));
              },
            },
          );
        }}
      />
    </div>
  );
}

/**
 * The teamspaces it may be added to and is not in yet: for its owner, those
 * they are in themselves, whatever their role in the workspace; for the
 * workspace's owners and admins, any but General, which it is in already.
 */
function AddToTeamspace({
  agent,
  own,
  onDone,
}: {
  agent: Agent;
  own: boolean;
  onDone: (message: string) => void;
}) {
  const workspace = useWorkspace();
  const teamspaces = useQuery(teamspacesQuery(workspace.id));
  const [teamspaceId, setTeamspaceId] = useState("");
  const add = useAddTeamspaceMember(workspace.id, teamspaceId);
  const open =
    teamspaces.data?.teamspaces.filter(
      (teamspace) =>
        teamspace.access !== "default" &&
        (own ? teamspace.role !== undefined && teamspace.role !== null : manages(workspace.role)),
    ) ?? [];
  const members = useQueries({
    queries: open.map((teamspace) => teamspaceMembersQuery(workspace.id, teamspace.id)),
  });
  const choices = open.filter(
    (_, index) => members[index]?.data?.some((member) => member.user.id === agent.id) !== true,
  );
  if (teamspaces.isPending) {
    return null;
  }
  return (
    <form
      noValidate
      aria-label={`${copy.teamspaces}: ${agent.name}`}
      className="grid gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        const teamspace = choices.find((choice) => choice.id === teamspaceId);
        if (teamspace === undefined) {
          return;
        }
        add.mutate(
          { user_id: agent.id, role: "member" },
          {
            onSuccess: () => {
              setTeamspaceId("");
              onDone(copy.added(agent.name, teamspace.name));
            },
          },
        );
      }}
    >
      <p className="text-sm font-medium">{copy.teamspaces}</p>
      {add.isError && <ErrorNotice error={add.error} />}
      {choices.length === 0 ? (
        <p className="text-sm text-muted-foreground">{copy.noTeamspace}</p>
      ) : (
        <div className="flex flex-wrap items-end gap-2">
          <span className="w-56">
            <NativeSelect
              aria-label={copy.teamspace}
              value={teamspaceId}
              onChange={(event) => setTeamspaceId(event.target.value)}
            >
              <option value="">{copy.teamspaceChoose}</option>
              {choices.map((teamspace) => (
                <option key={teamspace.id} value={teamspace.id}>
                  {teamspace.name}
                </option>
              ))}
            </NativeSelect>
          </span>
          <Button
            type="submit"
            variant="outline"
            size="sm"
            disabled={teamspaceId === "" || add.isPending}
          >
            {add.isPending ? copy.adding : copy.add}
          </Button>
        </div>
      )}
      <p className="text-xs text-muted-foreground">{copy.teamspacesHint}</p>
    </form>
  );
}

/** An agent's tokens: listed for its owner and the managers, made by its owner alone. */
function Tokens({ agent, own }: { agent: Agent; own: boolean }) {
  const workspace = useWorkspace();
  const tokens = useQuery(agentTokensQuery(workspace.id, agent.id));
  const [created, setCreated] = useState<CreatedApiToken>();
  const live = agent.status === "active";
  return (
    <section aria-label={copy.tokensLabel(agent.name)} className="grid gap-3">
      <p className="text-sm font-medium">{copy.tokens}</p>
      <p className="text-xs text-muted-foreground">{copy.tokensHint}</p>
      {created ? (
        <CreatedToken created={created} onDone={() => setCreated(undefined)} />
      ) : (
        own && live && <TokenForm agent={agent} onCreated={setCreated} />
      )}
      {tokens.isPending && <Loading />}
      {tokens.isError && (
        <ErrorNotice
          error={tokens.error}
          retrying={tokens.isFetching}
          onRetry={() => void tokens.refetch()}
        />
      )}
      {tokens.data?.length === 0 && (
        <p className="text-sm text-muted-foreground">{copy.noTokens}</p>
      )}
      {tokens.data !== undefined && tokens.data.length > 0 && (
        <ul className="grid gap-2">
          {tokens.data.map((token) => (
            <TokenRow key={token.id} agent={agent} token={token} />
          ))}
        </ul>
      )}
    </section>
  );
}

function TokenForm({
  agent,
  onCreated,
}: {
  agent: Agent;
  onCreated: (created: CreatedApiToken) => void;
}) {
  const workspace = useWorkspace();
  const create = useCreateAgentToken(workspace.id, agent.id);
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<ReadonlySet<AgentScope>>(() => new Set(["read", "write"]));
  const [problems, setProblems] = useState<{
    name?: string | undefined;
    scopes?: string | undefined;
  }>({});
  const scopesErrorId = useId();

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const chosen = AGENT_SCOPES.filter((scope) => scopes.has(scope));
    const found = {
      name: tokenNameProblem(name),
      scopes: chosen.length === 0 ? strings.validation.scopes : undefined,
    };
    setProblems(found);
    if (found.name !== undefined || found.scopes !== undefined) {
      focusFirstInvalid(event.currentTarget, found.name === undefined ? [] : ["name"]);
      return;
    }
    create.mutate(
      { name: name.trim(), scopes: chosen },
      {
        onSuccess: (token) => {
          setName("");
          onCreated(token);
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.makeToken}
      className="grid gap-3 rounded-md border border-border p-3"
    >
      {create.isError && <ErrorNotice error={create.error} />}
      <Field label={copy.tokenName} error={problems.name}>
        {(control) => (
          <Input
            {...control}
            name="name"
            autoComplete="off"
            placeholder={copy.tokenNamePlaceholder}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        )}
      </Field>
      <fieldset
        className="grid gap-1"
        {...(problems.scopes ? { "aria-describedby": scopesErrorId } : {})}
      >
        <legend className="mb-1 text-sm font-medium">{copy.scopes}</legend>
        {AGENT_SCOPES.map((scope) => (
          <CheckboxField
            key={scope}
            label={copy.scopeNames[scope]}
            hint={copy.scopeHints[scope]}
            checked={scopes.has(scope)}
            onCheckedChange={(on) =>
              setScopes((current) => {
                const next = new Set(current);
                if (on) {
                  next.add(scope);
                } else {
                  next.delete(scope);
                }
                return next;
              })
            }
          />
        ))}
        {problems.scopes && (
          <p id={scopesErrorId} className="text-sm text-bad">
            {problems.scopes}
          </p>
        )}
      </fieldset>
      <Button type="submit" size="sm" className="justify-self-start" disabled={create.isPending}>
        {create.isPending ? copy.makingToken : copy.makeToken}
      </Button>
    </form>
  );
}

function CreatedToken({ created, onDone }: { created: CreatedApiToken; onDone: () => void }) {
  return (
    <Alert live="polite">
      <AlertTitle className="flex items-center gap-2">
        <KeyRound aria-hidden className="size-4" />
        {copy.tokenMade(created.api_token.name)}
      </AlertTitle>
      <AlertDescription>
        <p>{copy.tokenOnce}</p>
        <CopyField label={copy.secret} value={created.token} />
        <div>
          <Button variant="outline" size="sm" onClick={onDone}>
            {copy.done}
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  );
}

function TokenRow({ agent, token }: { agent: Agent; token: ApiToken }) {
  const workspace = useWorkspace();
  const revoke = useRevokeAgentToken(workspace.id, agent.id);
  const [confirming, setConfirming] = useState(false);
  const tokens = strings.account.tokens;
  return (
    <li className="grid gap-2 rounded-md border border-border p-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-start">
      <div className="grid gap-1.5">
        <p className="text-sm font-medium">{token.name}</p>
        <ChipList aria-label={copy.scopes}>
          {token.scopes.map((scope) => (
            <Chip key={scope}>{tokens.scopeNames[scope]}</Chip>
          ))}
        </ChipList>
        <KeyValueList className="text-xs">
          <KeyValueRow label={tokens.prefix}>
            <span className="font-mono">{token.prefix}…</span>
          </KeyValueRow>
          <KeyValueRow label={tokens.made}>{formatDateTime(token.created_at)}</KeyValueRow>
          <KeyValueRow label={tokens.lastUsed}>
            {token.last_used_at ? formatDateTime(token.last_used_at) : tokens.never}
          </KeyValueRow>
        </KeyValueList>
        {revoke.isError && <ErrorNotice error={revoke.error} />}
      </div>
      <div>
        <Button
          variant="outline"
          size="sm"
          disabled={revoke.isPending}
          aria-label={copy.revokeLabel(token.name)}
          onClick={() => setConfirming(true)}
        >
          {copy.revoke}
        </Button>
      </div>
      <ConfirmDialog
        open={confirming}
        title={copy.revokeTitle(token.name)}
        description={copy.revokeLede}
        confirm={copy.revoke}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          revoke.mutate(token.id);
        }}
      />
    </li>
  );
}
