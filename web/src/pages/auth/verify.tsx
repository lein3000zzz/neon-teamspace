import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useCallback } from "react";
import { useVerifyEmail } from "@/api/auth";
import { AuthCard, authLink } from "@/components/auth/auth-layout";
import { ResendVerificationForm } from "@/components/auth/resend-verification";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { useLinkSecret } from "@/lib/links";
import { strings } from "@/lib/strings";
import { MissingToken, useSendOnce } from "./link-pages";

const copy = strings.auth.verify;

/**
 * Opened from the mailed link: verifies the address, then goes on to sign in
 * with it filled in. The password is still needed; the link alone signs
 * nobody in.
 */
export function VerifyPage() {
  const { token: fromAddress } = useSearch({ from: "/_auth/verify" });
  const token = useLinkSecret(fromAddress);
  const navigate = useNavigate();
  const { mutate, isError, error } = useVerifyEmail();

  const send = useCallback(
    (value: string) =>
      mutate(value, {
        onSuccess: (email) =>
          void navigate({ to: "/login", state: { notice: "verified", email }, replace: true }),
      }),
    [mutate, navigate],
  );
  useSendOnce(token, send);

  if (token === undefined) {
    return <MissingToken />;
  }
  if (isError) {
    return (
      <AuthCard
        title={strings.auth.link.brokenTitle}
        lede={copy.brokenLede}
        footer={
          <p>
            <Link to="/login" className={authLink}>
              {strings.auth.backToSignIn}
            </Link>
          </p>
        }
      >
        <ErrorNotice error={error} />
        <ResendVerificationForm />
      </AuthCard>
    );
  }
  return (
    <AuthCard title={copy.pending}>
      <Loading />
    </AuthCard>
  );
}
