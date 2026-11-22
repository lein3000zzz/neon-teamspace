import { useQuery } from "@tanstack/react-query";
import { Link, useLocation } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";
import { authConfigQuery, useRequestPasswordReset } from "@/api/auth";
import { AuthCard, authLink } from "@/components/auth/auth-layout";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Field } from "@/components/forms/field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { strings } from "@/lib/strings";
import { emailProblem } from "@/lib/validation";

const copy = strings.auth.forgot;

/**
 * Asks for a link that sets a new password. Afterwards it says the same
 * whether or not the address has an account, as the server does.
 */
export function ForgotPage() {
  const arrival = useLocation({ select: (location) => location.state });
  const config = useQuery(authConfigQuery);
  const request = useRequestPasswordReset();
  const [email, setEmail] = useState(arrival.email ?? "");
  const [problem, setProblem] = useState<string>();

  const footer = (
    <p>
      <Link to="/login" className={authLink}>
        {strings.auth.backToSignIn}
      </Link>
    </p>
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
  if (!config.data.password_reset) {
    return <AuthCard title={copy.offTitle} lede={copy.offLede} footer={footer} />;
  }
  if (request.isSuccess) {
    return <AuthCard title={copy.sentTitle} lede={copy.sentLede} footer={footer} />;
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const found = emailProblem(email);
    setProblem(found);
    if (found === undefined) {
      request.mutate(email.trim());
    } else {
      event.currentTarget.querySelector("input")?.focus();
    }
  };

  return (
    <AuthCard title={copy.title} lede={copy.lede} footer={footer}>
      <form noValidate onSubmit={submit} className="grid gap-4">
        {request.isError && <ErrorNotice error={request.error} />}
        <Field label={strings.auth.email} error={problem}>
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
        <Button type="submit" size="lg" disabled={request.isPending}>
          {request.isPending ? copy.pending : copy.submit}
        </Button>
      </form>
    </AuthCard>
  );
}
