import type { WorkspaceSummary } from "@/api/workspaces";
import { browserStorage } from "@/theme/preferences";

/** One entry per account, so people sharing a browser each come back to their own. */
const keyFor = (userId: string) => `neon.workspace.${userId}`;

/**
 * Notes the workspace someone is in, so the app opens on it next time, in this
 * browser. By id, which stays when the slug changes. Where storage is refused,
 * the app opens on the first workspace instead.
 */
export function rememberWorkspace(userId: string, workspaceId: string) {
  try {
    browserStorage()?.setItem(keyFor(userId), workspaceId);
  } catch {
    // Not remembered: the first workspace opens next time.
  }
}

export function lastWorkspaceId(userId: string): string | undefined {
  try {
    return browserStorage()?.getItem(keyFor(userId)) ?? undefined;
  } catch {
    return undefined;
  }
}

/** The workspace to open: the one last used while the user is still in it, else the first. */
export function pickWorkspace(
  workspaces: readonly WorkspaceSummary[],
  lastId: string | undefined,
): WorkspaceSummary | undefined {
  return workspaces.find((workspace) => workspace.id === lastId) ?? workspaces[0];
}
