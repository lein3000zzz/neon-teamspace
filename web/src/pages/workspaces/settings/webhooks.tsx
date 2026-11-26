import { useQueries, useQuery } from "@tanstack/react-query";
import { Link, useSearch } from "@tanstack/react-router";
import { KeyRound } from "lucide-react";
import { useId, useState, type FormEvent, type ReactNode } from "react";
import { agentsQuery } from "@/api/agents";
import { hasCode } from "@/api/errors";
import { teamspacePagesQuery, type TreePage } from "@/api/pages";
import { teamspacesQuery } from "@/api/teamspaces";
import {
  deliveriesQuery,
  incomingRequestsQuery,
  incomingWebhooksQuery,
  useCreateIncomingWebhook,
  useCreateWebhook,
  useDeleteIncomingWebhook,
  useDeleteWebhook,
  useRemoveIncomingSecret,
  useRotateIncomingSecret,
  useRotateIncomingUrl,
  useRotateWebhookSecret,
  useTestWebhook,
  useUpdateIncomingWebhook,
  useUpdateWebhook,
  webhooksQuery,
  WEBHOOK_EVENTS,
  type IncomingWebhook,
  type Webhook,
  type WebhookEvent,
} from "@/api/webhooks";
import { membersQuery, type Person } from "@/api/workspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { CopyField } from "@/components/feedback/copy-field";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { CheckboxField, Field, focusFirstInvalid } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { KindBadge, PersonLabel } from "@/components/people/person-label";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { useMe } from "@/components/shell/signed-in";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";
import { displayNameProblem, required } from "@/lib/validation";
import { manages } from "@/lib/workspace-rules";

const copy = strings.workspaces.webhooks;

/**
 * A workspace's webhooks: the outgoing ones the user may manage, each with its
 * test event, secret, log and switch; and, for owners and admins, the
 * incoming ones, each with its URL and signing secret, shown once, and its
 * requests. `?for=` keeps the outgoing ones of one identity, as an agent's
 * card links here. Each part says what was done in it above its list, so the
 * outgoing ones come first under the tabs, whose rule stands for theirs.
 */
export function WebhooksSettingsPage() {
  const workspace = useWorkspace();
  const search = useSearch({ from: "/_app/w/$slug/settings/webhooks" });
  return (
    <div className="grid gap-10">
      <Outgoing identity={search.for} />
      {manages(workspace.role) ? (
        <Incoming />
      ) : (
        <PageSection title={copy.incoming}>
          <p className="text-sm text-muted-foreground">{copy.incomingAdmins}</p>
        </PageSection>
      )}
    </div>
  );
}

/** A secret or URL, shown this once, with a Copy button. */
function ShownOnce({
  title,
  lede,
  fields,
  onDone,
}: {
  title: string;
  lede: string;
  fields: { label: string; value: string }[];
  onDone: () => void;
}) {
  return (
    <Alert live="polite">
      <AlertTitle className="flex items-center gap-2">
        <KeyRound aria-hidden className="size-4" />
        {title}
      </AlertTitle>
      <AlertDescription>
        <p>{lede}</p>
        {fields.map((field) => (
          <CopyField key={field.label} label={field.label} value={field.value} />
        ))}
        <div>
          <Button variant="outline" size="sm" onClick={onDone}>
            {copy.done}
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  );
}

/** Whom the user may register webhooks for: anyone for owners and admins, else themselves and theirs. */
function useIdentities(): Person[] {
  const workspace = useWorkspace();
  const me = useMe();
  const members = useQuery(membersQuery(workspace.id));
  const all = members.data?.map((member) => member.user) ?? [];
  if (manages(workspace.role)) {
    return all;
  }
  return all.filter((person) => person.id === me.id || person.owner?.id === me.id);
}

/** What was done in a part of the page, such as a webhook deleted, read out as it changes. */
function Done({ message }: { message: string | undefined }) {
  return (
    <output className="block text-sm text-muted-foreground empty:hidden">{message ?? ""}</output>
  );
}

function Outgoing({ identity }: { identity: string | undefined }) {
  const workspace = useWorkspace();
  const [done, setDone] = useState<string>();
  const webhooks = useQuery(webhooksQuery(workspace.id));
  const agents = useQuery(agentsQuery(workspace.id));
  const named = agents.data?.find((agent) => agent.id === identity)?.name;
  const shown =
    identity === undefined
      ? webhooks.data
      : webhooks.data?.filter((webhook) => webhook.identity.id === identity);
  return (
    <PageSection title={copy.outgoing} hint={copy.outgoingHint}>
      {workspace.role !== "guest" && <RegisterForm preset={identity} />}
      {identity !== undefined && (
        <p className="flex flex-wrap items-baseline gap-2 text-sm text-muted-foreground">
          {named !== undefined && copy.showing(named)}
          <Link
            to="/w/$slug/settings/webhooks"
            params={{ slug: workspace.slug }}
            className="underline"
          >
            {copy.showAll}
          </Link>
        </p>
      )}
      {webhooks.isPending && <Loading />}
      {webhooks.isError && (
        <ErrorNotice
          error={webhooks.error}
          retrying={webhooks.isFetching}
          onRetry={() => void webhooks.refetch()}
        />
      )}
      <Done message={done} />
      {shown?.length === 0 && <p className="text-sm text-muted-foreground">{copy.none}</p>}
      {shown !== undefined && shown.length > 0 && (
        <ul className="grid gap-3">
          {shown.map((webhook) => (
            <WebhookCard key={webhook.id} webhook={webhook} onDone={setDone} />
          ))}
        </ul>
      )}
    </PageSection>
  );
}

/** The events a form chose, in their order. */
function EventChoices({
  chosen,
  onChange,
  error,
}: {
  chosen: ReadonlySet<WebhookEvent>;
  onChange: (next: ReadonlySet<WebhookEvent>) => void;
  error: string | undefined;
}) {
  const errorId = useId();
  return (
    <fieldset className="grid gap-1" {...(error ? { "aria-describedby": errorId } : {})}>
      <legend className="mb-1 text-sm font-medium">{copy.events}</legend>
      {WEBHOOK_EVENTS.map((event) => (
        <CheckboxField
          key={event}
          label={copy.eventNames[event]}
          checked={chosen.has(event)}
          onCheckedChange={(on) => {
            const next = new Set(chosen);
            if (on) {
              next.add(event);
            } else {
              next.delete(event);
            }
            onChange(next);
          }}
        />
      ))}
      {error && (
        <p id={errorId} className="text-sm text-bad">
          {error}
        </p>
      )}
    </fieldset>
  );
}

function RegisterForm({ preset }: { preset: string | undefined }) {
  const workspace = useWorkspace();
  const identities = useIdentities();
  const create = useCreateWebhook(workspace.id);
  const [userId, setUserId] = useState(preset ?? "");
  const [url, setUrl] = useState("");
  const [events, setEvents] = useState<ReadonlySet<WebhookEvent>>(() => new Set(WEBHOOK_EVENTS));
  const [includeOwn, setIncludeOwn] = useState(false);
  const [problems, setProblems] = useState<{ user?: string; url?: string; events?: string }>({});
  const [secret, setSecret] = useState<string>();

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    create.reset();
    const chosen = WEBHOOK_EVENTS.filter((name) => events.has(name));
    const found = {
      ...(userId === "" ? { user: strings.validation.required } : {}),
      ...(required(url) === undefined ? {} : { url: strings.validation.required }),
      ...(chosen.length === 0 ? { events: copy.noEvent } : {}),
    };
    setProblems(found);
    if (Object.keys(found).length > 0) {
      focusFirstInvalid(
        event.currentTarget,
        [found.user ? "identity" : "", found.url ? "url" : ""].filter((name) => name !== ""),
      );
      return;
    }
    create.mutate(
      { user_id: userId, url: url.trim(), events: chosen, include_own: includeOwn },
      {
        onSuccess: (made) => {
          setUrl("");
          setSecret(made.secret);
        },
      },
    );
  };

  if (secret !== undefined) {
    return (
      <ShownOnce
        title={copy.registered}
        lede={copy.secretOnce}
        fields={[{ label: copy.secret, value: secret }]}
        onDone={() => setSecret(undefined)}
      />
    );
  }
  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.register}
      className="grid max-w-xl gap-4 rounded-lg border border-border bg-card p-4"
    >
      {create.isError &&
        (hasCode(create.error, "no_secret_key") ? (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">{copy.noSecretKey}</p>
          </Alert>
        ) : (
          <ErrorNotice error={create.error} />
        ))}
      <Field label={copy.identity} hint={copy.identityHint} error={problems.user}>
        {(control) => (
          <NativeSelect
            {...control}
            name="identity"
            value={userId}
            onChange={(event) => setUserId(event.target.value)}
          >
            <option value="">{copy.identityChoose}</option>
            {identities.map((person) => (
              <option key={person.id} value={person.id}>
                {person.owner
                  ? strings.people.via(person.display_name, person.owner.display_name)
                  : person.display_name}
              </option>
            ))}
          </NativeSelect>
        )}
      </Field>
      <Field label={copy.url} hint={copy.urlHint} error={problems.url}>
        {(control) => (
          <Input
            {...control}
            name="url"
            type="url"
            inputMode="url"
            autoComplete="off"
            spellCheck={false}
            placeholder="https://hooks.example.com/neon"
            value={url}
            onChange={(event) => setUrl(event.target.value)}
          />
        )}
      </Field>
      <EventChoices chosen={events} onChange={setEvents} error={problems.events} />
      <CheckboxField
        label={copy.includeOwn}
        hint={copy.includeOwnHint}
        checked={includeOwn}
        onCheckedChange={setIncludeOwn}
      />
      <Button type="submit" className="justify-self-start" disabled={create.isPending}>
        {create.isPending ? copy.pending : copy.submit}
      </Button>
    </form>
  );
}

/** What is open on a webhook's card. */
type Open = "edit" | "delete" | "rotate" | "log" | undefined;

function WebhookCard({ webhook, onDone }: { webhook: Webhook; onDone: (message: string) => void }) {
  const workspace = useWorkspace();
  const update = useUpdateWebhook(workspace.id);
  const remove = useDeleteWebhook(workspace.id);
  const rotate = useRotateWebhookSecret(workspace.id);
  const test = useTestWebhook(workspace.id);
  const [open, setOpen] = useState<Open>();
  const [secret, setSecret] = useState<string>();
  const failed = [update, remove, rotate, test].find((mutation) => mutation.isError);
  const toggle = webhook.enabled ? copy.turnOff : copy.turnOn;
  const owner = webhook.identity.display_name;

  return (
    <ItemCard aria-label={copy.cardLabel(owner, webhook.url)}>
      <div className="grid min-w-0 gap-2">
        <p className="font-medium">
          <PersonLabel person={webhook.identity} />
        </p>
        <p className="font-mono text-sm break-all">{webhook.url}</p>
        <ChipList>
          <Chip tone={webhook.enabled ? "strong" : "default"}>
            {webhook.enabled ? copy.on : copy.off}
          </Chip>
          {webhook.events.map((event) => (
            <Chip key={event}>{copy.eventNames[event]}</Chip>
          ))}
          {webhook.include_own && <Chip>{copy.ownToo}</Chip>}
        </ChipList>
        {!webhook.enabled && (
          <Alert tone="error" live="polite">
            <AlertTitle>{copy.offTitle}</AlertTitle>
            {webhook.disabled_reason && (
              <AlertDescription>
                <p>{webhook.disabled_reason}</p>
              </AlertDescription>
            )}
          </Alert>
        )}
        {webhook.enabled && webhook.failures > 0 && (
          <p className="text-sm text-bad">{copy.failures(webhook.failures)}</p>
        )}
        {webhook.previous_secret_expires_at && (
          <p className="text-xs text-muted-foreground">
            {copy.previousUntil(formatDateTime(webhook.previous_secret_expires_at))}
          </p>
        )}
        {webhook.created_by && (
          <p className="text-xs text-muted-foreground">
            {copy.madeBy(webhook.created_by.display_name)}
          </p>
        )}
        <output className="block text-sm text-muted-foreground empty:hidden">
          {test.data === undefined
            ? ""
            : test.data.outcome === "delivered"
              ? copy.tested.delivered(test.data.status_code, test.data.latency_ms)
              : copy.tested.failed(
                  copy.errorKinds[test.data.error_kind ?? ""] ?? test.data.error_kind ?? "",
                  test.data.status_code,
                )}
        </output>
        {failed?.error && <ErrorNotice error={failed.error} />}
        {secret !== undefined && (
          <ShownOnce
            title={copy.rotated}
            lede={copy.rotatedOnce}
            fields={[{ label: copy.secret, value: secret }]}
            onDone={() => setSecret(undefined)}
          />
        )}
        {open === "edit" && (
          <EditWebhook
            webhook={webhook}
            onClose={() => setOpen(undefined)}
            onDone={() => onDone(copy.saved)}
          />
        )}
        {open === "log" && <DeliveryLog webhook={webhook} />}
      </div>
      <ItemActions>
        <Button
          variant="outline"
          size="sm"
          disabled={test.isPending}
          aria-label={copy.testLabel(webhook.url)}
          onClick={() => {
            test.reset();
            test.mutate(webhook.id);
          }}
        >
          {test.isPending ? copy.testing : copy.test}
        </Button>
        <Button
          variant="outline"
          size="sm"
          aria-label={copy.logLabel(webhook.url)}
          aria-expanded={open === "log"}
          onClick={() => setOpen(open === "log" ? undefined : "log")}
        >
          {open === "log" ? copy.hideLog : copy.log}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={update.isPending}
          aria-label={copy.toggleLabel(toggle, webhook.url)}
          onClick={() =>
            update.mutate({ webhookId: webhook.id, body: { enabled: !webhook.enabled } })
          }
        >
          {toggle}
        </Button>
        <Button
          variant="outline"
          size="sm"
          aria-label={copy.editLabel(webhook.url)}
          onClick={() => setOpen("edit")}
        >
          {copy.edit}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={rotate.isPending}
          aria-label={copy.rotateLabel(webhook.url)}
          onClick={() => setOpen("rotate")}
        >
          {copy.rotate}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={remove.isPending}
          aria-label={copy.deleteLabel(webhook.url)}
          onClick={() => setOpen("delete")}
        >
          {copy.delete}
        </Button>
      </ItemActions>
      <ConfirmDialog
        open={open === "rotate"}
        title={copy.rotateTitle}
        description={copy.rotateLede}
        confirm={copy.rotate}
        onCancel={() => setOpen(undefined)}
        onConfirm={() => {
          setOpen(undefined);
          rotate.mutate(webhook.id, { onSuccess: (made) => setSecret(made.secret) });
        }}
      />
      <ConfirmDialog
        open={open === "delete"}
        title={copy.deleteTitle}
        description={copy.deleteLede}
        confirm={copy.delete}
        onCancel={() => setOpen(undefined)}
        onConfirm={() => {
          setOpen(undefined);
          remove.mutate(webhook.id, { onSuccess: () => onDone(copy.deleted) });
        }}
      />
    </ItemCard>
  );
}

function EditWebhook({
  webhook,
  onClose,
  onDone,
}: {
  webhook: Webhook;
  onClose: () => void;
  onDone: () => void;
}) {
  const workspace = useWorkspace();
  const update = useUpdateWebhook(workspace.id);
  const [url, setUrl] = useState(webhook.url);
  const [events, setEvents] = useState<ReadonlySet<WebhookEvent>>(() => new Set(webhook.events));
  const [includeOwn, setIncludeOwn] = useState(webhook.include_own);
  const [problems, setProblems] = useState<{ url?: string; events?: string }>({});

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const chosen = WEBHOOK_EVENTS.filter((name) => events.has(name));
    const found = {
      ...(required(url) === undefined ? {} : { url: strings.validation.required }),
      ...(chosen.length === 0 ? { events: copy.noEvent } : {}),
    };
    setProblems(found);
    if (Object.keys(found).length > 0) {
      focusFirstInvalid(event.currentTarget, found.url ? ["url"] : []);
      return;
    }
    update.mutate(
      {
        webhookId: webhook.id,
        body: {
          ...(url.trim() === webhook.url ? {} : { url: url.trim() }),
          events: chosen,
          include_own: includeOwn,
        },
      },
      {
        onSuccess: () => {
          onDone();
          onClose();
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.editLabel(webhook.url)}
      className="grid gap-4 rounded-md border border-border p-3"
    >
      {update.isError && <ErrorNotice error={update.error} />}
      <Field label={copy.url} error={problems.url}>
        {(control) => (
          <Input
            {...control}
            name="url"
            type="url"
            spellCheck={false}
            value={url}
            onChange={(event) => setUrl(event.target.value)}
          />
        )}
      </Field>
      <EventChoices chosen={events} onChange={setEvents} error={problems.events} />
      <CheckboxField label={copy.includeOwn} checked={includeOwn} onCheckedChange={setIncludeOwn} />
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

/** A small table of a log, a row per entry, as cards on a phone. */
function LogList({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section aria-label={label} className="grid gap-2">
      <ul className="grid gap-1.5">{children}</ul>
    </section>
  );
}

function DeliveryLog({ webhook }: { webhook: Webhook }) {
  const workspace = useWorkspace();
  const log = useQuery(deliveriesQuery(workspace.id, webhook.id));
  if (log.isPending) {
    return <Loading />;
  }
  if (log.isError) {
    return <ErrorNotice error={log.error} onRetry={() => void log.refetch()} />;
  }
  if (log.data.length === 0) {
    return <p className="text-sm text-muted-foreground">{copy.noAttempts}</p>;
  }
  return (
    <LogList label={copy.logLabel(webhook.url)}>
      {log.data.map((attempt) => (
        <li
          key={attempt.id}
          className="grid gap-1 rounded-md border border-border px-3 py-2 text-xs"
        >
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <Chip tone={attempt.outcome === "delivered" ? "strong" : "default"}>
              {copy.outcomes[attempt.outcome]}
            </Chip>
            <span className="font-mono">{attempt.event_type}</span>
            <span className="text-muted-foreground">{copy.attempt(attempt.attempt)}</span>
            {attempt.status_code !== null && attempt.status_code !== undefined && (
              <span className="font-mono">{attempt.status_code}</span>
            )}
            {attempt.latency_ms !== null && attempt.latency_ms !== undefined && (
              <span className="text-muted-foreground">{copy.latency(attempt.latency_ms)}</span>
            )}
            <span className="text-muted-foreground">{formatDateTime(attempt.at)}</span>
          </p>
          {attempt.error_kind && (
            <p className="text-muted-foreground">
              {copy.errorKinds[attempt.error_kind] ?? attempt.error_kind}
            </p>
          )}
          {attempt.excerpt && (
            <p className="font-mono break-all whitespace-pre-wrap text-muted-foreground">
              {attempt.excerpt}
            </p>
          )}
        </li>
      ))}
    </LogList>
  );
}

/** A database or page the user may pick as a target, with its teamspace and depth. */
type Choice = { page: TreePage; teamspace: string; depth: number };

/** The databases and pages of the teamspaces the user sees, each before those under it. */
function useTargets(): { choices: Choice[]; pending: boolean } {
  const workspace = useWorkspace();
  const teamspaces = useQuery(teamspacesQuery(workspace.id));
  const list = teamspaces.data?.teamspaces ?? [];
  const trees = useQueries({
    queries: list.map((teamspace) => teamspacePagesQuery(workspace.id, teamspace.id)),
  });
  const choices: Choice[] = [];
  list.forEach((teamspace, index) => {
    const pages = trees[index]?.data ?? [];
    const depth = new Map<string, number>();
    for (const page of pages) {
      const level = page.parent_id ? (depth.get(page.parent_id) ?? 0) + 1 : 0;
      depth.set(page.id, level);
      if (page.kind !== "row" && page.access === "edit") {
        choices.push({ page, teamspace: teamspace.name, depth: level });
      }
    }
  });
  return { choices, pending: teamspaces.isPending || trees.some((tree) => tree.isPending) };
}

function Incoming() {
  const workspace = useWorkspace();
  const hooks = useQuery(incomingWebhooksQuery(workspace.id));
  const [done, setDone] = useState<string>();
  return (
    <PageSection title={copy.incoming} hint={copy.incomingHint}>
      <MakeIncoming />
      {hooks.isPending && <Loading />}
      {hooks.isError && (
        <ErrorNotice
          error={hooks.error}
          retrying={hooks.isFetching}
          onRetry={() => void hooks.refetch()}
        />
      )}
      <Done message={done} />
      {hooks.data?.length === 0 && (
        <p className="text-sm text-muted-foreground">{copy.noneIncoming}</p>
      )}
      {hooks.data !== undefined && hooks.data.length > 0 && (
        <ul className="grid gap-3">
          {hooks.data.map((hook) => (
            <IncomingCard key={hook.id} hook={hook} onDone={setDone} />
          ))}
        </ul>
      )}
    </PageSection>
  );
}

function MakeIncoming() {
  const workspace = useWorkspace();
  const create = useCreateIncomingWebhook(workspace.id);
  const { choices, pending } = useTargets();
  const [name, setName] = useState("");
  const [target, setTarget] = useState("");
  const [signed, setSigned] = useState(false);
  const [problems, setProblems] = useState<{ name?: string; target?: string }>({});
  const [made, setMade] = useState<{ name: string; url: string; secret?: string }>();

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    create.reset();
    const nameProblem = required(name) ?? displayNameProblem(name);
    const found = {
      ...(nameProblem === undefined ? {} : { name: nameProblem }),
      ...(target === "" ? { target: strings.validation.required } : {}),
    };
    setProblems(found);
    if (Object.keys(found).length > 0) {
      focusFirstInvalid(
        event.currentTarget,
        [found.name ? "name" : "", found.target ? "target" : ""].filter((field) => field !== ""),
      );
      return;
    }
    create.mutate(
      { name: name.trim(), target_id: target, signed },
      {
        onSuccess: (created) => {
          setName("");
          setTarget("");
          setSigned(false);
          setMade({
            name: created.hook.name,
            url: created.url,
            ...(created.signing_secret ? { secret: created.signing_secret } : {}),
          });
        },
      },
    );
  };

  if (made !== undefined) {
    return (
      <ShownOnce
        title={copy.made(made.name)}
        lede={copy.urlOnce}
        fields={[
          { label: copy.urlLabel, value: made.url },
          ...(made.secret === undefined ? [] : [{ label: copy.secret, value: made.secret }]),
        ]}
        onDone={() => setMade(undefined)}
      />
    );
  }
  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.make}
      className="grid max-w-xl gap-4 rounded-lg border border-border bg-card p-4"
    >
      {create.isError &&
        (hasCode(create.error, "agent_limit") ? (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">{copy.agentLimit}</p>
          </Alert>
        ) : hasCode(create.error, "no_secret_key") ? (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">{copy.noSecretKey}</p>
          </Alert>
        ) : (
          <ErrorNotice error={create.error} />
        ))}
      <Field label={copy.name} hint={copy.nameHint} error={problems.name}>
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
      <Field label={copy.target} hint={copy.targetHint} error={problems.target}>
        {(control) =>
          !pending && choices.length === 0 ? (
            <p id={control.id} className="text-sm text-muted-foreground">
              {copy.noTargets}
            </p>
          ) : (
            <NativeSelect
              {...control}
              name="target"
              value={target}
              onChange={(event) => setTarget(event.target.value)}
            >
              <option value="">{copy.targetChoose}</option>
              {choices.map(({ page, teamspace, depth }) => (
                <option key={page.id} value={page.id}>
                  {`${"  ".repeat(depth)}${page.title || strings.pages.untitled} · ${
                    copy.kinds[page.kind === "database" ? "database" : "page"]
                  } · ${teamspace}`}
                </option>
              ))}
            </NativeSelect>
          )
        }
      </Field>
      <CheckboxField
        label={copy.signed}
        hint={copy.signedHint}
        checked={signed}
        onCheckedChange={setSigned}
      />
      <Button type="submit" className="justify-self-start" disabled={create.isPending}>
        {create.isPending ? copy.making : copy.makeSubmit}
      </Button>
    </form>
  );
}

/** What is open on an incoming webhook's card. */
type OpenIncoming = "rename" | "url" | "sign" | "unsign" | "delete" | "requests" | undefined;

function IncomingCard({
  hook,
  onDone,
}: {
  hook: IncomingWebhook;
  onDone: (message: string) => void;
}) {
  const workspace = useWorkspace();
  const update = useUpdateIncomingWebhook(workspace.id);
  const remove = useDeleteIncomingWebhook(workspace.id);
  const rotateUrl = useRotateIncomingUrl(workspace.id);
  const sign = useRotateIncomingSecret(workspace.id);
  const unsign = useRemoveIncomingSecret(workspace.id);
  const [open, setOpen] = useState<OpenIncoming>();
  const [shown, setShown] = useState<{ title: string; label: string; value: string }>();
  const failed = [update, remove, rotateUrl, sign, unsign].find((mutation) => mutation.isError);
  const toggle = hook.enabled ? copy.turnOff : copy.turnOn;
  const status = hook.integration.status;

  return (
    <ItemCard aria-label={hook.name}>
      <div className="grid min-w-0 gap-2">
        <p className="flex min-w-0 flex-wrap items-center gap-2 font-medium">
          <span className="break-words">{hook.name}</span>
          <KindBadge kind="integration" />
        </p>
        <p className="text-sm">
          {copy.kinds[hook.target.kind]}: {hook.target.title ?? copy.unseenTarget}
        </p>
        <ChipList>
          <Chip tone={hook.enabled && status === "active" ? "strong" : "default"}>
            {hook.enabled ? copy.on : copy.off}
          </Chip>
          <Chip>{hook.signed ? copy.signedChip : copy.unsignedChip}</Chip>
          <Chip>{strings.workspaces.agents.statuses[status]}</Chip>
          {hook.integration.owner && (
            <Chip>{strings.workspaces.agents.owner(hook.integration.owner.display_name)}</Chip>
          )}
        </ChipList>
        {status === "suspended" && <p className="text-sm text-bad">{copy.suspended}</p>}
        {status === "deleted" && <p className="text-sm text-bad">{copy.integrationDeleted}</p>}
        {!hook.enabled && (
          <Alert tone="error" live="polite">
            <AlertTitle>{copy.offTitle}</AlertTitle>
          </Alert>
        )}
        {hook.previous_secret_expires_at && (
          <p className="text-xs text-muted-foreground">
            {copy.previousUntil(formatDateTime(hook.previous_secret_expires_at))}
          </p>
        )}
        {failed?.error && <ErrorNotice error={failed.error} />}
        {shown !== undefined && (
          <ShownOnce
            title={shown.title}
            lede={shown.label === copy.urlLabel ? copy.urlOnce : copy.rotatedOnce}
            fields={[{ label: shown.label, value: shown.value }]}
            onDone={() => setShown(undefined)}
          />
        )}
        {open === "rename" && (
          <RenameIncoming
            hook={hook}
            onClose={() => setOpen(undefined)}
            onDone={(name) => onDone(copy.renamed(name))}
          />
        )}
        {open === "requests" && <RequestLog hook={hook} />}
      </div>
      <ItemActions>
        <Button
          variant="outline"
          size="sm"
          aria-label={copy.requestsLabel(hook.name)}
          aria-expanded={open === "requests"}
          onClick={() => setOpen(open === "requests" ? undefined : "requests")}
        >
          {open === "requests" ? copy.hideRequests : copy.requests}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={update.isPending}
          aria-label={copy.toggleLabel(toggle, hook.name)}
          onClick={() => update.mutate({ hookId: hook.id, body: { enabled: !hook.enabled } })}
        >
          {toggle}
        </Button>
        <Button
          variant="outline"
          size="sm"
          aria-label={copy.renameLabel(hook.name)}
          onClick={() => setOpen("rename")}
        >
          {copy.rename}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={rotateUrl.isPending}
          aria-label={copy.newUrlLabel(hook.name)}
          onClick={() => setOpen("url")}
        >
          {copy.newUrl}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={sign.isPending}
          aria-label={copy.signLabel(hook.name)}
          onClick={() => setOpen("sign")}
        >
          {hook.signed ? copy.rotateSigning : copy.sign}
        </Button>
        {hook.signed && (
          <Button
            variant="outline"
            size="sm"
            disabled={unsign.isPending}
            aria-label={copy.unsignLabel(hook.name)}
            onClick={() => setOpen("unsign")}
          >
            {copy.unsign}
          </Button>
        )}
        <Button
          variant="outline"
          size="sm"
          disabled={remove.isPending}
          aria-label={copy.deleteIncomingLabel(hook.name)}
          onClick={() => setOpen("delete")}
        >
          {copy.delete}
        </Button>
      </ItemActions>
      <ConfirmDialog
        open={open === "url"}
        title={copy.newUrlTitle(hook.name)}
        description={copy.newUrlLede}
        confirm={copy.newUrl}
        onCancel={() => setOpen(undefined)}
        onConfirm={() => {
          setOpen(undefined);
          rotateUrl.mutate(hook.id, {
            onSuccess: (made) =>
              setShown({
                title: copy.newUrlMade(hook.name),
                label: copy.urlLabel,
                value: made.url,
              }),
          });
        }}
      />
      <ConfirmDialog
        open={open === "sign"}
        title={copy.signTitle(hook.name)}
        description={copy.signLede}
        confirm={hook.signed ? copy.rotateSigning : copy.sign}
        onCancel={() => setOpen(undefined)}
        onConfirm={() => {
          setOpen(undefined);
          sign.mutate(hook.id, {
            onSuccess: (made) =>
              setShown({
                title: copy.signMade(hook.name),
                label: copy.secret,
                value: made.signing_secret,
              }),
          });
        }}
      />
      <ConfirmDialog
        open={open === "unsign"}
        title={copy.unsignTitle(hook.name)}
        description={copy.unsignLede}
        confirm={copy.unsign}
        onCancel={() => setOpen(undefined)}
        onConfirm={() => {
          setOpen(undefined);
          unsign.mutate(hook.id);
        }}
      />
      <ConfirmDialog
        open={open === "delete"}
        title={copy.deleteIncomingTitle(hook.name)}
        description={copy.deleteIncomingLede}
        confirm={copy.delete}
        onCancel={() => setOpen(undefined)}
        onConfirm={() => {
          setOpen(undefined);
          remove.mutate(hook.id, { onSuccess: () => onDone(copy.deletedIncoming(hook.name)) });
        }}
      />
    </ItemCard>
  );
}

function RenameIncoming({
  hook,
  onClose,
  onDone,
}: {
  hook: IncomingWebhook;
  onClose: () => void;
  onDone: (name: string) => void;
}) {
  const workspace = useWorkspace();
  const update = useUpdateIncomingWebhook(workspace.id);
  const [name, setName] = useState(hook.name);
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
      { hookId: hook.id, body: { name: name.trim() } },
      {
        onSuccess: (saved) => {
          onDone(saved.name);
          onClose();
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.renameLabel(hook.name)}
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

function RequestLog({ hook }: { hook: IncomingWebhook }) {
  const workspace = useWorkspace();
  const log = useQuery(incomingRequestsQuery(workspace.id, hook.id));
  if (log.isPending) {
    return <Loading />;
  }
  if (log.isError) {
    return <ErrorNotice error={log.error} onRetry={() => void log.refetch()} />;
  }
  if (log.data.length === 0) {
    return <p className="text-sm text-muted-foreground">{copy.noRequests}</p>;
  }
  return (
    <LogList label={copy.requestsLabel(hook.name)}>
      {log.data.map((request) => (
        <li
          key={request.id}
          className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-md border border-border px-3 py-2 text-xs"
        >
          <Chip tone={request.outcome === "ok" ? "strong" : "default"}>{request.outcome}</Chip>
          <span className="font-mono">{request.status_code}</span>
          {request.action && <span className="font-mono">{request.action}</span>}
          <span className="text-muted-foreground">{request.client_ip}</span>
          <span className="text-muted-foreground">{copy.latency(request.latency_ms)}</span>
          <span className="text-muted-foreground">{formatDateTime(request.at)}</span>
        </li>
      ))}
    </LogList>
  );
}
