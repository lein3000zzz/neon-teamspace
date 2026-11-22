import { useQuery } from "@tanstack/react-query";
import { Link, useLocation, useSearch } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";
import { meQuery } from "@/api/account";
import { authConfigQuery, useLogin } from "@/api/auth";
import { hasCode } from "@/api/errors";
import { AuthCard, AuthFooterLine, authLink } from "@/components/auth/auth-layout";
import { ResendVerification } from "@/components/auth/resend-verification";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { safeNext, useLeaveFor } from "@/lib/links";
import { strings } from "@/lib/strings";
import { emailProblem, required } from "@/lib/validation";

const copy = strings.auth.signIn;

type Problems = { email?: string | undefined; password?: string | undefined };

/**
 * Email and password. Once signed in, on to the page that sent the visitor
 * here, or home. The answer to a wrong password never says which half was
 * wrong; a disabled or unverified account is only named after the right one.
 */
export function LoginPage() {
  const { next } = useSearch({ from: "/_auth/login" });
  const arrival = useLocation({ select: (location) => location.state });
  const me = useQuery(meQuery);
  const config = useQuery(authConfigQuery);
  const login = useLogin();
  const [email, setEmail] = useState(arrival.email ?? "");
  const [password, setPassword] = useState("");
  const [problems, setProblems] = useState<Problems>({});

  useLeaveFor(!!me.data, safeNext(next));
  if (me.data) {
    return null;
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const found: Problems = { email: emailProblem(email), password: required(password) };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    login.mutate({ email: email.trim(), password });
  };

  const unverified = hasCode(login.error, "email_not_verified");
  const notice = arrival.notice === "verified" ? copy.verified : copy.passwordSet;

  return (
    <AuthCard
      title={copy.title}
      footer={
        config.data && (
          <>
            {config.data.registration !== "closed" && (
              <AuthFooterLine question={copy.noAccount}>
                <Link to="/register" className={authLink}>
                  {copy.createOne}
                </Link>
              </AuthFooterLine>
            )}
            {config.data.requests && (
              <AuthFooterLine question={copy.noInvite}>
                <Link to="/join" className={authLink}>
                  {copy.askForOne}
                </Link>
              </AuthFooterLine>
            )}
          </>
        )
      }
    >
      <form noValidate onSubmit={submit} className="grid gap-4">
        {arrival.notice !== undefined && !login.isError && (
          <Alert live="polite">
            <p className="text-foreground">{notice}</p>
          </Alert>
        )}
        {login.isError && (
          <ErrorNotice error={login.error}>
            {unverified && <ResendVerification email={email} />}
          </ErrorNotice>
        )}

        <Field label={strings.auth.email} error={problems.email}>
          {(control) => (
            <Input
              {...control}
              name="email"
              type="email"
              autoComplete="username"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
            />
          )}
        </Field>

        <Field
          label={strings.auth.password}
          error={problems.password}
          aside={
            config.data?.password_reset && (
              <Link to="/forgot" state={{ email: email.trim() }} className={authLink}>
                {copy.forgot}
              </Link>
            )
          }
        >
          {(control) => (
            <Input
              {...control}
              name="password"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
          )}
        </Field>

        <Button type="submit" size="lg" disabled={login.isPending}>
          {login.isPending ? copy.pending : copy.submit}
        </Button>
      </form>
    </AuthCard>
  );
}
