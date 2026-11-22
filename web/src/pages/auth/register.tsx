import { useQuery } from "@tanstack/react-query";
import { Link, useSearch } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";
import { meQuery } from "@/api/account";
import { authConfigQuery, inviteQuery, useRegister, type AuthConfig } from "@/api/auth";
import { ApiError, hasCode } from "@/api/errors";
import { AuthCard, AuthFooterLine, authLink } from "@/components/auth/auth-layout";
import { ResendVerification } from "@/components/auth/resend-verification";
import { ErrorNotice, Loading, errorText } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useLeaveFor, useLinkSecret } from "@/lib/links";
import { strings } from "@/lib/strings";
import { displayNameProblem, emailProblem, passwordProblem, required } from "@/lib/validation";

const copy = strings.auth.register;

/**
 * Creating an account, as the registration mode allows: a form for anyone
 * while open, the same with an invite code while by invite, and a word on
 * whom to ask while closed. The configuration decides, so the page waits for
 * it before showing any form.
 */
export function RegisterPage() {
  const { invite } = useSearch({ from: "/_auth/register" });
  const code = useLinkSecret(invite);
  const me = useQuery(meQuery);
  const config = useQuery(authConfigQuery);

  useLeaveFor(!!me.data, "/");
  if (me.data) {
    return null;
  }
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
  if (config.data.registration === "closed") {
    return <AuthCard title={copy.closedTitle} lede={copy.closedLede} footer={<SignInLine />} />;
  }
  return <RegisterForm config={config.data} linkCode={code} />;
}

function SignInLine() {
  return (
    <AuthFooterLine question={copy.haveAccount}>
      <Link to="/login" className={authLink}>
        {copy.signIn}
      </Link>
    </AuthFooterLine>
  );
}

type Problems = {
  email?: string | undefined;
  display_name?: string | undefined;
  password?: string | undefined;
  invite_code?: string | undefined;
};

/** Which field a refusal from the server is about. */
const FIELD_OF: Record<string, keyof Problems> = {
  invalid_email: "email",
  email_taken: "email",
  weak_password: "password",
  invalid_invite: "invite_code",
};

function RegisterForm({ config, linkCode }: { config: AuthConfig; linkCode: string | undefined }) {
  const byInvite = config.registration === "invite";
  const register = useRegister();
  const [email, setEmail] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState("");
  const [inviteCode, setInviteCode] = useState(linkCode ?? "");
  const [problems, setProblems] = useState<Problems>({});

  // An invite from a link is looked up as the page opens: it may be bound to one
  // address, which the form then fills in and keeps.
  const lookup = useQuery({ ...inviteQuery(linkCode ?? ""), enabled: byInvite && !!linkCode });
  const bound = lookup.data ?? undefined;
  const address = bound ?? email;

  const result = register.data;
  if (result?.kind === "verify") {
    return (
      <AuthCard title={copy.checkInbox} lede={copy.sent(result.email)} footer={<SignInLine />}>
        <ResendVerification email={result.email} />
      </AuthCard>
    );
  }
  if (result?.kind === "created") {
    return (
      <AuthCard title={copy.created} lede={copy.createdLede(result.email)}>
        <ErrorNotice error={result.error} />
        <Button asChild size="lg">
          <Link to="/login" state={{ email: result.email }}>
            {copy.signIn}
          </Link>
        </Button>
      </AuthCard>
    );
  }
  if (result?.kind === "signed-in") {
    // Signed in: the page above moves on.
    return null;
  }

  const serverField =
    register.error instanceof ApiError ? FIELD_OF[register.error.code] : undefined;
  const shown: Problems = {
    ...problems,
    ...(serverField === undefined ? {} : { [serverField]: errorText(register.error) }),
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    register.reset();
    const found: Problems = {
      email: emailProblem(address),
      display_name: displayNameProblem(displayName),
      password: passwordProblem(password, config.min_password_length),
      invite_code: byInvite ? required(inviteCode) : undefined,
    };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    const name = displayName.trim();
    register.mutate({
      email: address.trim(),
      password,
      ...(name === "" ? {} : { display_name: name }),
      ...(byInvite ? { invite_code: inviteCode.trim() } : {}),
    });
  };

  return (
    <AuthCard
      title={copy.title}
      lede={byInvite ? copy.inviteLede : undefined}
      footer={
        <>
          <SignInLine />
          {byInvite && config.requests && (
            <AuthFooterLine question={strings.auth.signIn.noInvite}>
              <Link to="/join" className={authLink}>
                {strings.auth.signIn.askForOne}
              </Link>
            </AuthFooterLine>
          )}
        </>
      }
    >
      <form noValidate onSubmit={submit} className="grid gap-4">
        {lookup.isError && (
          <ErrorNotice
            error={lookup.error}
            {...(hasCode(lookup.error, "invalid_invite") ? { title: copy.inviteBroken } : {})}
          />
        )}
        {register.isError && serverField === undefined && <ErrorNotice error={register.error} />}

        <Field
          label={strings.auth.email}
          error={shown.email}
          hint={bound === undefined ? undefined : copy.emailBound}
        >
          {(control) => (
            <Input
              {...control}
              name="email"
              type="email"
              autoComplete="email"
              readOnly={bound !== undefined}
              value={address}
              onChange={(event) => setEmail(event.target.value)}
            />
          )}
        </Field>

        <Field label={copy.displayName} hint={copy.displayNameHint} error={shown.display_name}>
          {(control) => (
            <Input
              {...control}
              name="display_name"
              autoComplete="name"
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
            />
          )}
        </Field>

        <Field
          label={strings.auth.password}
          hint={copy.passwordHint(config.min_password_length)}
          error={shown.password}
        >
          {(control) => (
            <Input
              {...control}
              name="password"
              type="password"
              autoComplete="new-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
          )}
        </Field>

        {byInvite && (
          <Field
            label={copy.inviteCode}
            hint={linkCode === undefined ? undefined : copy.inviteFromLink}
            error={shown.invite_code}
          >
            {(control) => (
              <Input
                {...control}
                name="invite_code"
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
                className="font-mono"
                value={inviteCode}
                onChange={(event) => setInviteCode(event.target.value)}
              />
            )}
          </Field>
        )}

        <Button type="submit" size="lg" disabled={register.isPending}>
          {register.isPending ? copy.pending : copy.submit}
        </Button>
      </form>
    </AuthCard>
  );
}
