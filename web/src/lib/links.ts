import { useRouter } from "@tanstack/react-router";
import { useEffect, useState } from "react";

/**
 * Goes to `href`, in place of this history entry, once `when` turns true: a
 * signed-in visitor on a sign-in page moves on. From an effect rather than a
 * <Navigate>, so a page that renders again while the move is pending does not
 * start it again.
 */
export function useLeaveFor(when: boolean, href: string) {
  const router = useRouter();
  useEffect(() => {
    if (when) {
      // A whole path with its search goes as an href, which the router reads in
      // place of `to`.
      void router.navigate({ to: "/", href, replace: true });
    }
  }, [when, href, router]);
}

/**
 * Where to go after signing in: the `next` the sign-in page was sent with,
 * when it is a page of this app. Anything that resolves to another origin,
 * such as //evil.example or a tab smuggled into /\t/evil.example, goes home
 * instead, so a crafted link cannot send people elsewhere.
 */
export function safeNext(next: string | undefined): string {
  if (next === undefined || !next.startsWith("/")) {
    return "/";
  }
  const url = new URL(next, window.location.origin);
  if (url.origin !== window.location.origin || url.pathname === "/login") {
    return "/";
  }
  return url.pathname + url.search + url.hash;
}

/**
 * Takes a one-time secret, a mailed link's token or an invite code, out of
 * the address as the page opens: the page keeps it in memory, and the history
 * entry is replaced by the same path without it, so it is not left in the
 * browser's history, its address bar, or a bookmark.
 */
export function useLinkSecret(fromAddress: string | undefined): string | undefined {
  const [secret] = useState(fromAddress);
  const router = useRouter();

  useEffect(() => {
    if (fromAddress !== undefined) {
      router.history.replace(router.state.location.pathname, router.state.location.state);
    }
  }, [fromAddress, router]);

  return secret;
}

/** A search value as a page reads it: a string that is not empty, or nothing. */
export function searchText(value: unknown): string | undefined {
  return typeof value === "string" && value !== "" ? value : undefined;
}

/** The address an invite's code opens the sign-up form with. */
export function inviteLink(code: string): string {
  return `${window.location.origin}/register?invite=${encodeURIComponent(code)}`;
}
