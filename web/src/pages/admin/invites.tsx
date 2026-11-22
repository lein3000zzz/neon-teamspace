import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ChevronDown, ChevronRight, Plus } from "lucide-react";
import { useId, useState, type FormEvent } from "react";
import {
  inviteCheckQuery,
  invitesQuery,
  useCreateInvite,
  useInviteCode,
  useRevokeInvite,
  type CreatedInvite,
  type Invite,
} from "@/api/admin";
import { authConfigQuery } from "@/api/auth";
import { ApiError } from "@/api/errors";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { CopyField } from "@/components/feedback/copy-field";
import { ErrorNotice, errorText, Loading, sentence } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useDebounced } from "@/lib/debounce";
import { formatDateTime } from "@/lib/format";
import { inviteLink } from "@/lib/links";
import { strings } from "@/lib/strings";
import {
  dayProblem,
  emailProblem,
  endOfDay,
  inAYear,
  inviteCodeProblem,
  inviteUsesProblem,
  today,
} from "@/lib/validation";
import { CreatedInviteNotice } from "./created-invite";

const copy = strings.admin.invites;

/** Invites: making them, by code or by mail, and every one made so far with its sign-ups. */
export function InvitesPage() {
  const config = useQuery(authConfigQuery);
  const invites = useQuery(invitesQuery);
  const [created, setCreated] = useState<CreatedInvite>();

  return (
    <div className="grid gap-10">
      <PageSection
        title={copy.make}
        hint={
          <>
            {copy.makeHint}{" "}
            {config.data && (
              <>
                {copy.modes[config.data.registration]}{" "}
                <Link to="/admin/settings" className="text-foreground underline underline-offset-4">
                  {copy.changeMode}
                </Link>
                .
              </>
            )}
          </>
        }
      >
        {created ? (
          <CreatedInviteNotice created={created} onDone={() => setCreated(undefined)} />
        ) : (
          <div className="grid gap-6">
            <MakeInviteForm onCreated={setCreated} />
            {invites.data?.by_email && <MailInviteForm onCreated={setCreated} />}
          </div>
        )}
      </PageSection>

      <PageSection title={copy.list}>
        {invites.isPending && <Loading />}
        {invites.isError && (
          <ErrorNotice
            error={invites.error}
            retrying={invites.isFetching}
            onRetry={() => void invites.refetch()}
          />
        )}
        {invites.data?.invites.length === 0 && (
          <p className="text-sm text-muted-foreground">{copy.none}</p>
        )}
        {invites.data && invites.data.invites.length > 0 && (
          <ul className="grid gap-3">
            {invites.data.invites.map((invite) => (
              <InviteCard key={invite.id} invite={invite} />
            ))}
          </ul>
        )}
      </PageSection>
    </div>
  );
}

type Problems = {
  code?: string | undefined;
  max_uses?: string | undefined;
  expires?: string | undefined;
};

/** A code typed, checked as it is typed, or left to the server; its sign-ups; its end. */
function MakeInviteForm({ onCreated }: { onCreated: (invite: CreatedInvite) => void }) {
  const create = useCreateInvite();
  const [code, setCode] = useState("");
  const [uses, setUses] = useState("1");
  const [expires, setExpires] = useState("");
  const [problems, setProblems] = useState<Problems>({});
  const [bounds] = useState(() => ({ first: today(), last: inAYear() }));

  // The server says whether the code is free; asked once typing pauses, and only
  // for a code whose form is right.
  const typed = useDebounced(code.trim(), 300);
  const checkable = typed !== "" && inviteCodeProblem(typed) === undefined;
  const check = useQuery({ ...inviteCheckQuery(typed), enabled: checkable });
  const settled = typed === code.trim();
  const problem = settled && checkable ? (check.data ?? undefined) : undefined;
  const taken = problem === undefined ? undefined : sentence(problem);
  const free = settled && checkable && check.isSuccess && check.data === null;

  const error = create.error;
  const codeRefused =
    error instanceof ApiError &&
    (error.code === "invite_code_taken" || error.code === "invalid_input")
      ? errorText(error)
      : undefined;
  const codeProblem = problems.code ?? taken ?? codeRefused;

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    create.reset();
    const found: Problems = {
      code: inviteCodeProblem(code),
      max_uses: inviteUsesProblem(uses),
      expires: dayProblem(expires, bounds.last),
    };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    const trimmed = code.trim();
    create.mutate(
      {
        max_uses: Number(uses),
        ...(trimmed === "" ? {} : { code: trimmed }),
        ...(expires === "" ? {} : { expires_at: endOfDay(expires) }),
      },
      {
        onSuccess: (invite) => {
          setCode("");
          setUses("1");
          setExpires("");
          onCreated(invite);
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.make}
      className="grid gap-4 rounded-lg border border-border bg-card p-4 sm:grid-cols-2"
    >
      {create.isError && codeRefused === undefined && (
        <div className="sm:col-span-2">
          <ErrorNotice error={error} />
        </div>
      )}
      <Field
        className="sm:col-span-2"
        label={copy.code}
        hint={codeProblem === undefined ? (free ? copy.codeFree : copy.codeHint) : undefined}
        error={codeProblem}
      >
        {(control) => (
          <Input
            {...control}
            name="code"
            className="font-mono"
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            placeholder={copy.codePlaceholder}
            value={code}
            onChange={(event) => setCode(event.target.value)}
          />
        )}
      </Field>
      <Field label={copy.uses} hint={copy.usesHint} error={problems.max_uses}>
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
      <Field label={copy.expires} hint={copy.expiresHint} error={problems.expires}>
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
      <div className="sm:col-span-2">
        <Button type="submit" disabled={create.isPending || taken !== undefined}>
          <Plus aria-hidden />
          {create.isPending ? copy.pending : copy.submit}
        </Button>
      </div>
    </form>
  );
}

/** Mailed, an invite is for its address alone, once. */
function MailInviteForm({ onCreated }: { onCreated: (invite: CreatedInvite) => void }) {
  const create = useCreateInvite();
  const [email, setEmail] = useState("");
  const [problem, setProblem] = useState<string>();
  const refused =
    create.error instanceof ApiError && create.error.code === "email_taken"
      ? errorText(create.error)
      : undefined;

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    create.reset();
    const found = emailProblem(email);
    setProblem(found);
    if (found !== undefined) {
      event.currentTarget.querySelector("input")?.focus();
      return;
    }
    create.mutate(
      { email: email.trim() },
      {
        onSuccess: (invite) => {
          setEmail("");
          onCreated(invite);
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.mail}
      className="grid gap-4 rounded-lg border border-border bg-card p-4"
    >
      {create.isError && refused === undefined && <ErrorNotice error={create.error} />}
      <Field label={copy.mail} hint={copy.mailHint} error={problem ?? refused}>
        {(control) => (
          <Input
            {...control}
            name="email"
            type="email"
            autoComplete="off"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
          />
        )}
      </Field>
      <div>
        <Button type="submit" variant="outline" disabled={create.isPending}>
          {create.isPending ? copy.pending : copy.mailSubmit}
        </Button>
      </div>
    </form>
  );
}

function InviteCard({ invite }: { invite: Invite }) {
  const revoke = useRevokeInvite();
  const reveal = useInviteCode();
  const [confirming, setConfirming] = useState(false);
  const [showSignups, setShowSignups] = useState(false);
  const signupsId = useId();
  const made = formatDateTime(invite.created_at);
  const open = invite.status === "open";
  const bound = invite.email ?? undefined;

  return (
    <ItemCard>
      <div className="grid min-w-0 gap-2">
        <ChipList>
          <Chip tone={open ? "strong" : "default"}>{copy.statuses[invite.status]}</Chip>
          <Chip>{bound === undefined ? copy.anyone : copy.forEmail(bound)}</Chip>
        </ChipList>
        <p className="text-sm">{copy.used(invite.uses, invite.max_uses)}</p>
        <p className="text-sm text-muted-foreground">
          {copy.madeAt(made)} · {copy.expiresAt(formatDateTime(invite.expires_at))}
        </p>

        {invite.signups.length > 0 && (
          <div className="grid gap-1">
            <Button
              variant="ghost"
              size="sm"
              className="justify-self-start"
              aria-expanded={showSignups}
              aria-controls={signupsId}
              onClick={() => setShowSignups((shown) => !shown)}
            >
              {showSignups ? <ChevronDown aria-hidden /> : <ChevronRight aria-hidden />}
              {copy.signups(invite.signups.length)}
            </Button>
            <ul id={signupsId} hidden={!showSignups} className="grid gap-1 pl-2 text-sm">
              {invite.signups.map((signup) => (
                <li
                  key={`${signup.used_at}-${signup.user_id ?? signup.email ?? ""}`}
                  className="flex flex-wrap gap-x-3"
                >
                  <span>{signup.email ?? copy.deleted}</span>
                  <span className="text-muted-foreground">{formatDateTime(signup.used_at)}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        {open && !invite.code_kept && (
          <p className="text-xs text-muted-foreground">{copy.notKept}</p>
        )}
        {reveal.data !== undefined && (
          <CopyField label={copy.link} value={inviteLink(reveal.data)} />
        )}
        {reveal.isError && <ErrorNotice error={reveal.error} />}
        {revoke.isError && <ErrorNotice error={revoke.error} />}
      </div>

      {open && (
        <ItemActions>
          {invite.code_kept && reveal.data === undefined && (
            <Button
              variant="outline"
              size="sm"
              disabled={reveal.isPending}
              aria-label={copy.showLinkLabel(made)}
              onClick={() => reveal.mutate(invite.id)}
            >
              {copy.showLink}
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={revoke.isPending}
            aria-label={copy.revokeLabel(made)}
            onClick={() => setConfirming(true)}
          >
            {copy.revoke}
          </Button>
        </ItemActions>
      )}

      <ConfirmDialog
        open={confirming}
        title={copy.revokeTitle}
        description={copy.revokeLede}
        confirm={copy.revoke}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          revoke.mutate(invite.id);
        }}
      />
    </ItemCard>
  );
}
