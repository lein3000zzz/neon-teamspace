import { useQuery } from "@tanstack/react-query";
import { KeyRound } from "lucide-react";
import { useId, useState, type FormEvent } from "react";
import {
  tokensQuery,
  useCreateToken,
  useRevokeToken,
  type ApiToken,
  type Scope,
} from "@/api/account";
import type { Schemas } from "@/api/client";
import { workspacesQuery, type WorkspaceSummary } from "@/api/workspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { CopyField } from "@/components/feedback/copy-field";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { CheckboxField, Field, focusFirstInvalid } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { useMe } from "@/components/shell/signed-in";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";
import { dayProblem, endOfDay, today, tokenNameProblem } from "@/lib/validation";
import { manages } from "@/lib/workspace-rules";

const copy = strings.account.tokens;

const SCOPES: readonly Scope[] = ["read", "write", "admin"];

/** Tokens for scripts and tools: made here, shown once, revoked here. */
export function TokensPage() {
  const tokens = useQuery(tokensQuery);
  const [created, setCreated] = useState<Schemas["CreatedApiToken"]>();

  return (
    <div className="grid gap-10">
      <PageSection title={copy.title} hint={copy.hint}>
        {created ? (
          <CreatedToken created={created} onDone={() => setCreated(undefined)} />
        ) : (
          <CreateTokenForm onCreated={setCreated} />
        )}
      </PageSection>

      <section aria-label={copy.title} className="grid gap-3">
        {tokens.isPending && <Loading />}
        {tokens.isError && (
          <ErrorNotice
            error={tokens.error}
            retrying={tokens.isFetching}
            onRetry={() => void tokens.refetch()}
          />
        )}
        {tokens.data?.length === 0 && <p className="text-sm text-muted-foreground">{copy.none}</p>}
        {tokens.data && tokens.data.length > 0 && (
          <ul className="grid gap-3">
            {tokens.data.map((token) => (
              <TokenCard key={token.id} token={token} />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

type Problems = {
  name?: string | undefined;
  scopes?: string | undefined;
  expires?: string | undefined;
};

function CreateTokenForm({
  onCreated,
}: {
  onCreated: (token: Schemas["CreatedApiToken"]) => void;
}) {
  const me = useMe();
  const create = useCreateToken();
  const workspaces = useQuery(workspacesQuery);
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<ReadonlySet<Scope>>(() => new Set(["read"]));
  const [expires, setExpires] = useState("");
  const [workspaceId, setWorkspaceId] = useState("");
  const [problems, setProblems] = useState<Problems>({});
  const bound = workspaces.data?.workspaces.find((workspace) => workspace.id === workspaceId);
  // Admin is the admin routes' across the instance, or the workspace's own admin work.
  const admin = bound === undefined ? me.role === "admin" : manages(bound.role);
  const offered = SCOPES.filter((scope) => scope !== "admin" || admin);
  const scopesErrorId = useId();

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

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const chosen = offered.filter((scope) => scopes.has(scope));
    const found: Problems = {
      name: tokenNameProblem(name),
      scopes: chosen.length === 0 ? strings.validation.scopes : undefined,
      expires: dayProblem(expires),
    };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    create.mutate(
      {
        name: name.trim(),
        scopes: chosen,
        ...(expires === "" ? {} : { expires_at: endOfDay(expires) }),
        ...(bound === undefined ? {} : { workspace_id: bound.id }),
      },
      {
        onSuccess: (token) => {
          setName("");
          setExpires("");
          setWorkspaceId("");
          setScopes(new Set(["read"]));
          onCreated(token);
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.create}
      className="grid max-w-xl gap-4 rounded-lg border border-border bg-card p-4"
    >
      <h3 className="text-sm font-semibold">{copy.create}</h3>
      {create.isError && <ErrorNotice error={create.error} />}
      <Field label={copy.name} error={problems.name}>
        {(control) => (
          <Input
            {...control}
            name="name"
            autoComplete="off"
            placeholder={copy.namePlaceholder}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        )}
      </Field>
      <Field label={copy.worksIn} hint={copy.worksInHint}>
        {(control) => (
          <NativeSelect
            {...control}
            name="workspace"
            value={workspaceId}
            onChange={(event) => setWorkspaceId(event.target.value)}
          >
            <option value="">{copy.everywhere}</option>
            {(workspaces.data?.workspaces ?? []).map((workspace) => (
              <option key={workspace.id} value={workspace.id}>
                {workspace.name}
              </option>
            ))}
          </NativeSelect>
        )}
      </Field>
      <fieldset
        className="grid gap-1"
        {...(problems.scopes ? { "aria-describedby": scopesErrorId } : {})}
      >
        <legend className="mb-1 text-sm font-medium">{copy.scopes}</legend>
        {offered.map((scope) => (
          <CheckboxField
            key={scope}
            label={copy.scopeNames[scope]}
            hint={
              scope === "admin" && bound !== undefined
                ? copy.boundAdminHint
                : copy.scopeHints[scope]
            }
            checked={scopes.has(scope)}
            onCheckedChange={(on) => toggle(scope, on)}
          />
        ))}
        {problems.scopes && (
          <p id={scopesErrorId} className="text-sm text-bad">
            {problems.scopes}
          </p>
        )}
      </fieldset>
      <Field label={copy.expires} hint={copy.expiresHint} error={problems.expires}>
        {(control) => (
          <Input
            {...control}
            name="expires"
            type="date"
            min={today()}
            value={expires}
            onChange={(event) => setExpires(event.target.value)}
          />
        )}
      </Field>
      <Button type="submit" className="justify-self-start" disabled={create.isPending}>
        {create.isPending ? copy.pending : copy.submit}
      </Button>
    </form>
  );
}

function CreatedToken({
  created,
  onDone,
}: {
  created: Schemas["CreatedApiToken"];
  onDone: () => void;
}) {
  return (
    <Alert live="polite" className="max-w-xl">
      <AlertTitle className="flex items-center gap-2">
        <KeyRound aria-hidden className="size-4" />
        {copy.createdTitle(created.api_token.name)}
      </AlertTitle>
      <AlertDescription>
        <p>{copy.createdLede}</p>
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

/**
 * Where a token works: every workspace, or the one it is bound to, by name
 * once the user's workspaces come; one they have left since is not among
 * them, and the token works nowhere until they are back.
 */
function worksIn(token: ApiToken, workspaces: readonly WorkspaceSummary[] | undefined): string {
  if (token.workspace_id === undefined || token.workspace_id === null) {
    return copy.everywhere;
  }
  if (workspaces === undefined) {
    return "…";
  }
  return workspaces.find((workspace) => workspace.id === token.workspace_id)?.name ?? copy.left;
}

function TokenCard({ token }: { token: ApiToken }) {
  const revoke = useRevokeToken();
  const workspaces = useQuery(workspacesQuery);
  const [confirming, setConfirming] = useState(false);

  return (
    <ItemCard>
      <div className="grid gap-2">
        <p className="font-medium">{token.name}</p>
        <ChipList aria-label={copy.scopes}>
          {token.scopes.map((scope) => (
            <Chip key={scope}>{copy.scopeNames[scope]}</Chip>
          ))}
        </ChipList>
        <KeyValueList className="text-sm">
          <KeyValueRow label={copy.prefix}>
            <span className="font-mono">{token.prefix}…</span>
          </KeyValueRow>
          <KeyValueRow label={copy.worksIn}>
            {worksIn(token, workspaces.data?.workspaces)}
          </KeyValueRow>
          <KeyValueRow label={copy.made}>{formatDateTime(token.created_at)}</KeyValueRow>
          <KeyValueRow label={copy.lastUsed}>
            {token.last_used_at ? formatDateTime(token.last_used_at) : copy.never}
          </KeyValueRow>
          <KeyValueRow label={copy.expiresAt}>
            {token.expires_at ? formatDateTime(token.expires_at) : copy.never}
          </KeyValueRow>
        </KeyValueList>
        {revoke.isError && <ErrorNotice error={revoke.error} />}
      </div>
      <ItemActions>
        <Button
          variant="outline"
          size="sm"
          disabled={revoke.isPending}
          aria-label={copy.revokeLabel(token.name)}
          onClick={() => setConfirming(true)}
        >
          {copy.revoke}
        </Button>
      </ItemActions>
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
    </ItemCard>
  );
}
