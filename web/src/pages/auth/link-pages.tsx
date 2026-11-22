import { Link } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { AuthCard, authLink } from "@/components/auth/auth-layout";
import { strings } from "@/lib/strings";

/**
 * Sends a mailed link's token once, as the page opens. A link works once, so
 * it must not go twice, however often the page mounts.
 */
export function useSendOnce(token: string | undefined, send: (token: string) => void) {
  const sent = useRef(false);
  useEffect(() => {
    if (token !== undefined && !sent.current) {
      sent.current = true;
      send(token);
    }
  }, [token, send]);
}

/** A link page opened without its token: cut short in the mail, or opened again from history. */
export function MissingToken() {
  return (
    <AuthCard
      title={strings.auth.link.missingTitle}
      lede={strings.auth.link.missingLede}
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
