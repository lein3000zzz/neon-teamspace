import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";
import { authConfigQuery, useResetPassword } from "@/api/auth";
import { hasCode } from "@/api/errors";
import { AuthCard, authLink } from "@/components/auth/auth-layout";
import { ErrorNotice, errorText, Loading } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useLinkSecret } from "@/lib/links";
import { strings } from "@/lib/strings";
import { passwordProblem } from "@/lib/validation";
import { ForgotPage } from "./forgot";

const copy = strings.auth.reset;

type Problems = { password?: string | undefined; repeat?: string | undefined };

/**
 * Opened from the mailed link: a new password, twice. The link is spent only
 * when the password is set, so a refused password can be tried again. Then
 * on to sign in with it, as every session was signed out. Without a token,
 * as the mail saying a password changed links here, it asks for an address to
 * mail a link to, like /forgot.
 */
export function ResetPage() {
  const { token: fromAddress } = useSearch({ from: "/_auth/reset" });
  const token = useLinkSecret(fromAddress);
  const navigate = useNavigate();
  const config = useQuery(authConfigQuery);
  const reset = useResetPassword();
  const [password, setPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const [problems, setProblems] = useState<Problems>({});

  if (token === undefined) {
    return <ForgotPage />;
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

  const minLength = config.data.min_password_length;
  const deadLink = hasCode(reset.error, "invalid_link");
  const weak = hasCode(reset.error, "weak_password") ? errorText(reset.error) : undefined;

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    reset.reset();
    const found: Problems = {
      password: passwordProblem(password, minLength),
      repeat: repeat === password ? undefined : strings.validation.passwordsDiffer,
    };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    reset.mutate(
      { token, password },
      {
        onSuccess: (email) =>
          void navigate({ to: "/login", state: { notice: "password-set", email }, replace: true }),
      },
    );
  };

  return (
    <AuthCard title={copy.title} lede={copy.lede}>
      <form noValidate onSubmit={submit} className="grid gap-4">
        {reset.isError && weak === undefined && (
          <ErrorNotice error={reset.error}>
            {deadLink && (
              <p>
                <Link to="/forgot" className={authLink}>
                  {copy.newLink}
                </Link>
              </p>
            )}
          </ErrorNotice>
        )}

        <Field
          label={copy.password}
          hint={strings.auth.register.passwordHint(minLength)}
          error={problems.password ?? weak}
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

        <Field label={copy.repeat} error={problems.repeat}>
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

        <Button type="submit" size="lg" disabled={reset.isPending || deadLink}>
          {reset.isPending ? copy.pending : copy.submit}
        </Button>
      </form>
    </AuthCard>
  );
}
