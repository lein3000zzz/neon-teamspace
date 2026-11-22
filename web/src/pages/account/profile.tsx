import { useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { useChangePassword, useUpdateProfile } from "@/api/account";
import { authConfigQuery } from "@/api/auth";
import { ApiError } from "@/api/errors";
import { ErrorNotice, errorText } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Chip } from "@/components/nodes/chip";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { PageSection } from "@/components/settings/page-section";
import { useMe } from "@/components/shell/signed-in";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { formatDate } from "@/lib/format";
import { strings } from "@/lib/strings";
import { displayNameProblem, passwordProblem, required } from "@/lib/validation";

/** Who you are here, and your password. */
export function ProfilePage() {
  return (
    <div className="grid gap-10">
      <ProfileSection />
      <PasswordSection />
    </div>
  );
}

function ProfileSection() {
  const copy = strings.account.profile;
  const me = useMe();
  const update = useUpdateProfile();
  const [name, setName] = useState(me.display_name);
  const [problem, setProblem] = useState<string>();

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    update.reset();
    const found = displayNameProblem(name);
    setProblem(found);
    if (found !== undefined) {
      event.currentTarget.querySelector("input")?.focus();
      return;
    }
    update.mutate(
      { display_name: name.trim() },
      { onSuccess: (user) => setName(user.display_name) },
    );
  };

  const refused =
    update.error instanceof ApiError && update.error.code === "invalid_input"
      ? errorText(update.error)
      : undefined;

  return (
    <PageSection title={copy.title} hint={copy.hint}>
      <KeyValueList className="max-w-xl text-sm">
        <KeyValueRow label={copy.email}>
          <span className="flex flex-wrap items-center gap-2">
            {me.email}
            <Chip>{me.email_verified_at ? copy.verified : copy.notVerified}</Chip>
          </span>
        </KeyValueRow>
        <KeyValueRow label={copy.role}>{copy.roles[me.role]}</KeyValueRow>
        <KeyValueRow label={copy.since}>{formatDate(me.created_at)}</KeyValueRow>
      </KeyValueList>

      <form noValidate onSubmit={submit} className="grid max-w-md gap-4">
        {update.isError && refused === undefined && <ErrorNotice error={update.error} />}
        <Field label={copy.displayName} hint={copy.displayNameHint} error={problem ?? refused}>
          {(control) => (
            <Input
              {...control}
              name="display_name"
              autoComplete="name"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          )}
        </Field>
        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" disabled={update.isPending}>
            {update.isPending ? copy.saving : copy.save}
          </Button>
          <output className="block text-sm text-muted-foreground">
            {update.isSuccess ? copy.saved : ""}
          </output>
        </div>
      </form>
    </PageSection>
  );
}

type PasswordProblems = {
  current_password?: string | undefined;
  new_password?: string | undefined;
  repeat?: string | undefined;
};

function PasswordSection() {
  const copy = strings.account.password;
  const config = useQuery(authConfigQuery);
  const change = useChangePassword();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [repeat, setRepeat] = useState("");
  const [problems, setProblems] = useState<PasswordProblems>({});
  const minLength = config.data?.min_password_length;

  const error = change.error;
  const serverProblems: PasswordProblems =
    error instanceof ApiError
      ? error.code === "invalid_credentials"
        ? { current_password: copy.wrong }
        : error.code === "weak_password"
          ? { new_password: errorText(error) }
          : {}
      : {};
  const shown = { ...problems, ...serverProblems };
  const general = change.isError && Object.keys(serverProblems).length === 0;

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    change.reset();
    const found: PasswordProblems = {
      current_password: required(current),
      new_password: passwordProblem(next, minLength ?? 1),
      repeat: repeat === next ? undefined : strings.validation.passwordsDiffer,
    };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof PasswordProblems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    change.mutate(
      { current_password: current, new_password: next },
      {
        onSuccess: () => {
          setCurrent("");
          setNext("");
          setRepeat("");
        },
      },
    );
  };

  return (
    <PageSection title={copy.title} hint={copy.hint}>
      <form noValidate onSubmit={submit} className="grid max-w-md gap-4">
        {general && <ErrorNotice error={error} />}
        <Field label={copy.current} error={shown.current_password}>
          {(control) => (
            <Input
              {...control}
              name="current_password"
              type="password"
              autoComplete="current-password"
              value={current}
              onChange={(event) => setCurrent(event.target.value)}
            />
          )}
        </Field>
        <Field
          label={copy.next}
          hint={minLength === undefined ? undefined : strings.auth.register.passwordHint(minLength)}
          error={shown.new_password}
        >
          {(control) => (
            <Input
              {...control}
              name="new_password"
              type="password"
              autoComplete="new-password"
              value={next}
              onChange={(event) => setNext(event.target.value)}
            />
          )}
        </Field>
        <Field label={copy.repeat} error={shown.repeat}>
          {(control) => (
            <Input
              {...control}
              name="repeat"
              type="password"
              autoComplete="new-password"
              value={repeat}
              onChange={(event) => setRepeat(event.target.value)}
            />
          )}
        </Field>
        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" disabled={change.isPending}>
            {change.isPending ? copy.pending : copy.submit}
          </Button>
          <output className="block text-sm text-muted-foreground">
            {change.isSuccess ? copy.done : ""}
          </output>
        </div>
      </form>
    </PageSection>
  );
}
