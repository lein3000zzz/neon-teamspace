import { Link, useSearch } from "@tanstack/react-router";
import { useUnsubscribe } from "@/api/inbox";
import { hasCode } from "@/api/errors";
import { AuthCard, authLink } from "@/components/auth/auth-layout";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Button } from "@/components/ui/button";
import { useLinkSecret } from "@/lib/links";
import { strings } from "@/lib/strings";

const copy = strings.unsubscribe;

function SettingsLink() {
  return (
    <p>
      <Link to="/account/notifications" className={authLink}>
        {copy.settings}
      </Link>
    </p>
  );
}

/**
 * Opened from the link at the foot of a notification mail: stops them, for
 * whoever the link was signed for, without signing in. It waits for a click,
 * as a mail scanner that opens the link must not stop anything; a mail client's
 * own unsubscribe button posts to the API at once instead.
 */
export function UnsubscribePage() {
  const { token: fromAddress } = useSearch({ from: "/_auth/unsubscribe" });
  const token = useLinkSecret(fromAddress);
  const unsubscribe = useUnsubscribe();

  if (token === undefined) {
    return <AuthCard title={copy.missingTitle} lede={copy.missingLede} footer={<SettingsLink />} />;
  }
  if (unsubscribe.isSuccess) {
    return <AuthCard title={copy.doneTitle} lede={copy.doneLede} footer={<SettingsLink />} />;
  }
  if (unsubscribe.isError && hasCode(unsubscribe.error, "invalid_token")) {
    return <AuthCard title={copy.brokenTitle} lede={copy.brokenLede} footer={<SettingsLink />} />;
  }
  return (
    <AuthCard title={copy.title} lede={copy.lede} footer={<SettingsLink />}>
      {unsubscribe.isError && <ErrorNotice error={unsubscribe.error} />}
      <Button disabled={unsubscribe.isPending} onClick={() => unsubscribe.mutate(token)}>
        {unsubscribe.isPending ? copy.stopping : copy.confirm}
      </Button>
    </AuthCard>
  );
}
