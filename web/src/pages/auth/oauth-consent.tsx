import { useQuery } from "@tanstack/react-query";
import { Link, useSearch } from "@tanstack/react-router";
import { useId, useState, type ReactNode } from "react";
import { meQuery, type Scope } from "@/api/account";
import { ApiError } from "@/api/errors";
import {
  AUTHORIZATION_PARAMS,
  consentQuery,
  useApprove,
  useDeny,
  type AuthorizationParams,
  type ConsentInfo,
  type ConsentOption,
} from "@/api/oauth";
import { AuthCard, authLink } from "@/components/auth/auth-layout";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { CheckboxField, Field } from "@/components/forms/field";
import { HueDot } from "@/components/nodes/hue-dot";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/native-select";
import { leaveFor } from "@/lib/leave";
import { strings } from "@/lib/strings";

const copy = strings.consent;

/** Whom the client acts as: the person, or one of their agents by id. */
type ActAs = { kind: "person" } | { kind: "agent"; id: string };

/**
 * Where an OAuth client's authorization request lands: the person signed in
 * chooses the workspace, whom the client acts as and its scopes, then allows
 * or denies it, and the browser goes back to the client either way. A request
 * that does not hold shows its problem, and never goes back to the client on
 * its own: anyone may register one, with any address.
 */
export function ConsentPage() {
  const search = useSearch({ from: "/_auth/oauth/consent" });
  if (search.error !== undefined) {
    return <RequestProblem code={search.error} />;
  }
  const request: AuthorizationParams = {};
  for (const key of AUTHORIZATION_PARAMS) {
    const value = search[key];
    if (value !== undefined) {
      request[key] = value;
    }
  }
  return <Consent request={request} />;
}

function Consent({ request }: { request: AuthorizationParams }) {
  const consent = useQuery(consentQuery(request));

  if (consent.isPending) {
    return (
      <AuthCard title={copy.loadingTitle}>
        <Loading label={copy.loading} />
      </AuthCard>
    );
  }
  if (consent.isError) {
    const { error } = consent;
    if (error instanceof ApiError && error.status === 400) {
      return <RequestProblem code={error.code} />;
    }
    return (
      <AuthCard title={copy.loadingTitle}>
        <ErrorNotice
          error={error}
          retrying={consent.isFetching}
          onRetry={() => void consent.refetch()}
        />
      </AuthCard>
    );
  }
  return <ConsentForm request={request} consent={consent.data} />;
}

/** A request that does not hold, named by its problem's code alone. */
function RequestProblem({ code }: { code: string }) {
  return (
    <AuthCard
      title={copy.problemTitle}
      lede={copy.problems[code] ?? copy.unknownProblem}
      footer={
        <>
          <p>{copy.problemFooter}</p>
          <p>
            <Link to="/" className={authLink}>
              {copy.home}
            </Link>
          </p>
        </>
      }
    />
  );
}

/** Whether a workspace offers the client anything to act as. */
function offers(option: ConsentOption): boolean {
  return Boolean(option.person_scopes) || (option.agents_on && option.agents.length > 0);
}

/** The first identity a workspace offers: the person, else their first agent. */
function firstIdentity(option: ConsentOption | undefined): ActAs | undefined {
  if (option === undefined) {
    return undefined;
  }
  if (option.person_scopes) {
    return { kind: "person" };
  }
  const agent = option.agents_on ? option.agents[0] : undefined;
  return agent === undefined ? undefined : { kind: "agent", id: agent.id };
}

/** The scopes offered for an identity in a workspace. */
function offeredFor(option: ConsentOption | undefined, actAs: ActAs | undefined): Scope[] {
  if (option === undefined || actAs === undefined) {
    return [];
  }
  return actAs.kind === "person" ? (option.person_scopes ?? []) : option.agent_scopes;
}

/**
 * The scopes ticked to start with: those the client asked for that are
 * offered, or read and write when it asked for none. Read always, as every
 * connection reads.
 */
function startingScopes(asked: readonly Scope[], offered: readonly Scope[]): Set<Scope> {
  const wanted: readonly Scope[] = asked.length === 0 ? ["read", "write"] : asked;
  return new Set<Scope>(["read", ...offered.filter((scope) => wanted.includes(scope))]);
}

function ConsentForm({ request, consent }: { request: AuthorizationParams; consent: ConsentInfo }) {
  const me = useQuery(meQuery);
  const approve = useApprove();
  const deny = useDeny();
  const usable = consent.workspaces.filter(offers);
  const [workspaceId, setWorkspaceId] = useState(usable[0]?.id ?? "");
  const option = usable.find((workspace) => workspace.id === workspaceId);
  const [actAs, setActAs] = useState(() => firstIdentity(option));
  const offered = offeredFor(option, actAs);
  const [scopes, setScopes] = useState(() =>
    startingScopes(consent.requested_scopes, offeredFor(option, firstIdentity(option))),
  );
  const [leaving, setLeaving] = useState(false);
  const actAsId = useId();
  const name = consent.client.name;
  const busy = approve.isPending || deny.isPending || leaving;

  const choose = (next: ActAs | undefined, inWorkspace: ConsentOption | undefined) => {
    setActAs(next);
    setScopes(startingScopes(consent.requested_scopes, offeredFor(inWorkspace, next)));
  };
  const go = ({ redirect_to }: { redirect_to: string }) => {
    setLeaving(true);
    leaveFor(redirect_to);
  };

  if (leaving) {
    return (
      <AuthCard title={copy.leavingTitle} lede={copy.leavingLede(consent.redirect_host)}>
        <Loading />
      </AuthCard>
    );
  }
  if (usable.length === 0) {
    return (
      <AuthCard
        title={copy.noWorkspaceTitle}
        lede={copy.noWorkspaceLede}
        footer={<DenyLine busy={busy} onDeny={() => deny.mutate(request, { onSuccess: go })} />}
      >
        {deny.isError && <ErrorNotice error={deny.error} />}
      </AuthCard>
    );
  }

  const toggle = (scope: Scope, on: boolean) =>
    setScopes((current) => {
      const next = new Set(current);
      if (on) {
        next.add(scope);
      } else {
        next.delete(scope);
      }
      return next;
    });
  const allow = () => {
    if (option === undefined || actAs === undefined) {
      return;
    }
    approve.mutate(
      {
        request,
        workspace_id: option.id,
        ...(actAs.kind === "agent" ? { agent_id: actAs.id } : {}),
        scopes: offered.filter((scope) => scopes.has(scope)),
      },
      { onSuccess: go },
    );
  };

  return (
    <AuthCard
      title={copy.title(name)}
      lede={copy.lede(name)}
      footer={
        <>
          <p>{copy.goesTo(consent.redirect_host)}</p>
          {me.data && <p>{copy.signedInAs(me.data.email)}</p>}
          <p>{copy.revokeLater}</p>
        </>
      }
    >
      <Alert live="polite">
        <p className="text-foreground">{copy.named(name)}</p>
      </Alert>

      <Field label={copy.workspace}>
        {(control) => (
          <NativeSelect
            {...control}
            name="workspace"
            value={workspaceId}
            disabled={busy}
            onChange={(event) => {
              const next = usable.find((workspace) => workspace.id === event.target.value);
              setWorkspaceId(event.target.value);
              choose(firstIdentity(next), next);
            }}
          >
            {usable.map((workspace) => (
              <option key={workspace.id} value={workspace.id}>
                {workspace.name}
              </option>
            ))}
          </NativeSelect>
        )}
      </Field>

      {option && (
        <fieldset className="grid gap-2" aria-describedby={`${actAsId}-note`}>
          <legend className="mb-1 text-sm font-medium">{copy.actAs}</legend>
          {option.person_scopes && me.data && (
            <IdentityChoice
              name={actAsId}
              checked={actAs?.kind === "person"}
              disabled={busy}
              label={
                <span className="flex items-center gap-2">
                  <HueDot hue={me.data.hue} className="size-1.5" />
                  {copy.asMe(me.data.display_name)}
                </span>
              }
              hint={copy.asMeHint}
              onChoose={() => choose({ kind: "person" }, option)}
            />
          )}
          {option.agents_on &&
            option.agents.map((agent) => (
              <IdentityChoice
                key={agent.id}
                name={actAsId}
                checked={actAs?.kind === "agent" && actAs.id === agent.id}
                disabled={busy}
                label={
                  <span className="flex items-center gap-2">
                    <HueDot hue={agent.hue} className="size-1.5" />
                    {copy.asAgent(agent.name)}
                  </span>
                }
                hint={copy.asAgentHint}
                onChoose={() => choose({ kind: "agent", id: agent.id }, option)}
              />
            ))}
          <p id={`${actAsId}-note`} className="text-xs text-muted-foreground empty:hidden">
            {agentsNote(option)}
          </p>
        </fieldset>
      )}

      <fieldset className="grid gap-1">
        <legend className="mb-1 text-sm font-medium">{copy.scopes}</legend>
        {offered.map((scope) => (
          <CheckboxField
            key={scope}
            label={copy.scopeNames[scope]}
            hint={copy.scopeHints[scope]}
            checked={scope === "read" || scopes.has(scope)}
            disabled={scope === "read" || busy}
            onCheckedChange={(on) => toggle(scope, on)}
          />
        ))}
      </fieldset>

      {approve.isError && <ErrorNotice error={approve.error} />}
      {deny.isError && <ErrorNotice error={deny.error} />}

      <div className="flex gap-2">
        <Button
          variant="outline"
          className="flex-1"
          disabled={busy}
          onClick={() => deny.mutate(request, { onSuccess: go })}
        >
          {deny.isPending ? copy.denying : copy.deny}
        </Button>
        <Button className="flex-1" disabled={busy || actAs === undefined} onClick={allow}>
          {approve.isPending ? copy.allowing : copy.allow}
        </Button>
      </div>
    </AuthCard>
  );
}

/** What the identities of a workspace leave to say: why no agent is offered, if none is. */
function agentsNote(option: ConsentOption): string {
  if (option.agents_on) {
    return option.agents.length === 0 ? copy.noAgents : "";
  }
  return option.role === "guest" ? copy.agentsGuest : copy.agentsOff;
}

/** One identity to act as, a radio with its label and hint. */
function IdentityChoice({
  name,
  checked,
  disabled,
  label,
  hint,
  onChoose,
}: {
  name: string;
  checked: boolean;
  disabled: boolean;
  label: ReactNode;
  hint: string;
  onChoose: () => void;
}) {
  const id = useId();
  return (
    <div className="flex min-h-(--tap) items-start gap-3">
      <input
        id={id}
        type="radio"
        name={name}
        checked={checked}
        disabled={disabled}
        onChange={onChoose}
        aria-describedby={`${id}-hint`}
        className="mt-[calc((var(--tap)-1rem)/2)] size-4 accent-primary"
      />
      <div className="grid min-h-(--tap) content-center gap-0.5">
        <label htmlFor={id} className="cursor-pointer text-sm font-medium">
          {label}
        </label>
        <p id={`${id}-hint`} className="text-xs text-muted-foreground">
          {hint}
        </p>
      </div>
    </div>
  );
}

function DenyLine({ busy, onDeny }: { busy: boolean; onDeny: () => void }) {
  return (
    <Button variant="outline" disabled={busy} onClick={onDeny}>
      {busy ? copy.denying : copy.deny}
    </Button>
  );
}
