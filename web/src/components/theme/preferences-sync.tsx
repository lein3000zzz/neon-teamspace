import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { preferencesQuery } from "@/api/account";
import { useTheme } from "@/theme/theme-provider";

/**
 * Once someone is signed in, their account's look replaces whatever this
 * browser had, and becomes its cache for the next first paint. A failed read
 * leaves the cached look as it is.
 */
export function PreferencesSync({ userId }: { userId: string }) {
  const { load } = useTheme();
  const { data } = useQuery(preferencesQuery(userId));

  useEffect(() => {
    if (data !== undefined) {
      load(data);
    }
  }, [data, load]);

  return null;
}
