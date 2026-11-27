import { useCallback, useSyncExternalStore } from "react";

/** From this width up, a row opens beside its database rather than in its place. */
export const WIDE_QUERY = "(min-width: 64rem)";

/**
 * A touch screen as the main pointer, as on a phone or a tablet: nothing
 * hovers, and a finger is too coarse for small handles.
 */
export const COARSE_QUERY = "(pointer: coarse)";

/**
 * Whether a media query matches, following it as it changes. False where the
 * browser cannot tell, as without matchMedia.
 */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (notify: () => void) => {
      const list = window.matchMedia?.(query);
      list?.addEventListener("change", notify);
      return () => list?.removeEventListener("change", notify);
    },
    [query],
  );
  return useSyncExternalStore(subscribe, () => window.matchMedia?.(query).matches ?? false);
}
