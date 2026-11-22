import { Link, useSearch } from "@tanstack/react-router";
import { useConfirmJoin } from "@/api/auth";
import { AuthCard, authLink } from "@/components/auth/auth-layout";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Button } from "@/components/ui/button";
import { useLinkSecret } from "@/lib/links";
import { strings } from "@/lib/strings";
import { MissingToken, useSendOnce } from "./link-pages";

const copy = strings.auth.joinConfirm;

/** Opened from the mailed link: confirms the request, which the admins then see. */
export function JoinConfirmPage() {
  const { token: fromAddress } = useSearch({ from: "/_auth/join/confirm" });
  const token = useLinkSecret(fromAddress);
  const { mutate, isSuccess, isError, error } = useConfirmJoin();
  useSendOnce(token, mutate);

  if (token === undefined) {
    return <MissingToken />;
  }
  if (isSuccess) {
    return (
      <AuthCard
        title={copy.doneTitle}
        lede={copy.doneLede}
        footer={
          <p>
            <Link to="/login" className={authLink}>
              {strings.auth.backToSignIn}
            </Link>
          </p>
        }
      />
    );
  }
  if (isError) {
    return (
      <AuthCard title={strings.auth.link.brokenTitle} lede={copy.brokenLede}>
        <ErrorNotice error={error} />
        <Button asChild variant="outline">
          <Link to="/join">{copy.askAgain}</Link>
        </Button>
      </AuthCard>
    );
  }
  return (
    <AuthCard title={copy.pending}>
      <Loading />
    </AuthCard>
  );
}
