import { useEffect } from "react";
import { useCurrentWorkspace } from "@/components/workspaces/workspace-context";
import { useTheme } from "@/theme/theme-provider";

/**
 * A workspace's hue is the accent of its members who chose none of their own:
 * the hue of the workspace the sidebar shows, the one in view or, outside any,
 * the one used last. In no workspace, the accent is the fallback blue again.
 * Signing out keeps the last hue, which the next sign-in replaces.
 */
export function WorkspaceAccent() {
  const { follow } = useTheme();
  const { list, current } = useCurrentWorkspace();
  const known = list.isSuccess;
  const hue = current?.hue;

  useEffect(() => {
    if (known) {
      follow(hue);
    }
  }, [known, hue, follow]);

  return null;
}
