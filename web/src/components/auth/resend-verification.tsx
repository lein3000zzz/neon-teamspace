import { useState, type FormEvent } from "react";
import { useResendVerification } from "@/api/auth";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Field } from "@/components/forms/field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { strings } from "@/lib/strings";
import { emailProblem } from "@/lib/validation";

const copy = strings.auth.resend;

/**
 * Mails the verification link again. Its answer, like the server's, is the
 * same whether or not the address has an account that needs one.
 */
export function ResendVerification({ email }: { email: string }) {
  const resend = useResendVerification();
  return (
    <div className="grid gap-2">
      <Button
        type="button"
        variant="outline"
        disabled={emailProblem(email) !== undefined || resend.isPending}
        onClick={() => resend.mutate(email.trim())}
      >
        {resend.isPending ? copy.pending : copy.send}
      </Button>
      {resend.isSuccess && (
        <output className="block text-sm text-muted-foreground">{copy.done}</output>
      )}
      {resend.isError && <ErrorNotice error={resend.error} />}
    </div>
  );
}

/** Asks for the address first, for a page opened from a link that no longer works. */
export function ResendVerificationForm() {
  const resend = useResendVerification();
  const [email, setEmail] = useState("");
  const [problem, setProblem] = useState<string>();

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const found = emailProblem(email);
    setProblem(found);
    if (found === undefined) {
      resend.mutate(email.trim());
    } else {
      event.currentTarget.querySelector("input")?.focus();
    }
  };

  return (
    <form noValidate onSubmit={submit} className="grid gap-4">
      {resend.isError && <ErrorNotice error={resend.error} />}
      {resend.isSuccess && (
        <output className="block text-sm text-muted-foreground">{copy.done}</output>
      )}
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
      <Button type="submit" variant="outline" disabled={resend.isPending}>
        {resend.isPending ? copy.pending : copy.send}
      </Button>
    </form>
  );
}
