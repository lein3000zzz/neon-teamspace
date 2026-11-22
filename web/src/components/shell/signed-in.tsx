import { useQuery } from "@tanstack/react-query";
import { useRouter, useRouterState } from "@tanstack/react-router";
import { createContext, use, useEffect, type ReactNode } from "react";
import { meQuery, type Me } from "@/api/account";
import { PreferencesSync } from "@/components/theme/preferences-sync";

const SignedInContext = createContext<Me | null>(null);

/**
 * The pages behind sign-in. The route's beforeLoad has already sent visitors
 * without a session to sign in; this keeps watching, so a session that ends
 * later (signed out, expired, revoked from another browser) lands there too,
 * with the way back to this page.
 */
export function SignedIn({ children }: { children: ReactNode }) {
  const me = useQuery(meQuery);
  const router = useRouter();
  // The page on screen, not the one being navigated to: while the way to sign
  // in is pending, that one is the sign-in page itself.
  const href = useRouterState({
    select: (state) => (state.resolvedLocation ?? state.location).href,
  });
  const signedOut = me.data === null;

  useEffect(() => {
    if (signedOut) {
      void router.navigate({ to: "/login", search: { next: href }, replace: true });
    }
  }, [signedOut, href, router]);

  if (me.data === null || me.data === undefined) {
    return null;
  }
  return (
    <SignedInContext value={me.data}>
      <PreferencesSync userId={me.data.id} />
      {children}
    </SignedInContext>
  );
}

/** The signed-in user, inside the pages behind sign-in. */
export function useMe(): Me {
  const me = use(SignedInContext);
  if (me === null) {
    throw new Error("useMe must be used inside <SignedIn>");
  }
  return me;
}
