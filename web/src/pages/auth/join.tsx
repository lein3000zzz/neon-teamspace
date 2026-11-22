import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";
import { authConfigQuery, useRequestToJoin } from "@/api/auth";
import { ApiError } from "@/api/errors";
import { AuthCard, AuthFooterLine, authLink } from "@/components/auth/auth-layout";
import { ErrorNotice, Loading, errorText } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { strings } from "@/lib/strings";
import { characters, emailProblem, noteProblem } from "@/lib/validation";

const copy = strings.auth.join;

type Problems = { email?: string | undefined; note?: string | undefined };

/**
 * Asking for an invite with a note, while requests are on. Whatever comes of
 * it, an address with an account, one asked already, one denied lately, the
 * page says the same thing afterwards, as the server answers the same.
 */
export function JoinPage() {
  const config = useQuery(authConfigQuery);
  const ask = useRequestToJoin();
  const [email, setEmail] = useState("");
  const [note, setNote] = useState("");
  const [problems, setProblems] = useState<Problems>({});

  const footer = (
    <AuthFooterLine question={copy.haveInvite}>
      <Link to="/register" className={authLink}>
        {copy.createAccount}
      </Link>
    </AuthFooterLine>
  );

  if (config.isPending) {
    return (
      <AuthCard title={copy.title}>
        <Loading />
      </AuthCard>
    );
  }
  if (config.isError) {
    return (
      <AuthCard title={copy.title}>
        <ErrorNotice
          error={config.error}
          retrying={config.isFetching}
          onRetry={() => void config.refetch()}
        />
      </AuthCard>
    );
  }
  if (!config.data.requests) {
    return <AuthCard title={copy.offTitle} lede={copy.offLede} footer={footer} />;
  }
  if (ask.isSuccess) {
    return <AuthCard title={copy.sentTitle} lede={copy.sentLede} footer={footer} />;
  }

  const emailRefused =
    ask.error instanceof ApiError && ask.error.code === "invalid_email"
      ? errorText(ask.error)
      : undefined;

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    ask.reset();
    const found: Problems = { email: emailProblem(email), note: noteProblem(note) };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    const trimmed = note.trim();
    ask.mutate({ email: email.trim(), ...(trimmed === "" ? {} : { note: trimmed }) });
  };

  return (
    <AuthCard title={copy.title} lede={copy.lede} footer={footer}>
      <form noValidate onSubmit={submit} className="grid gap-4">
        {ask.isError && emailRefused === undefined && <ErrorNotice error={ask.error} />}

        <Field label={strings.auth.email} error={problems.email ?? emailRefused}>
          {(control) => (
            <Input
              {...control}
              name="email"
              type="email"
              autoComplete="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
            />
          )}
        </Field>

        <Field label={copy.note} hint={copy.noteHint(characters(note))} error={problems.note}>
          {(control) => (
            <Textarea
              {...control}
              name="note"
              rows={4}
              placeholder={copy.notePlaceholder}
              value={note}
              onChange={(event) => setNote(event.target.value)}
            />
          )}
        </Field>

        <Button type="submit" size="lg" disabled={ask.isPending}>
          {ask.isPending ? copy.pending : copy.submit}
        </Button>
      </form>
    </AuthCard>
  );
}
