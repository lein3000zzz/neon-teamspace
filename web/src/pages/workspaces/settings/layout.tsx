import { Outlet } from "@tanstack/react-router";
import { PageHeader } from "@/components/shell/page-header";
import { afterSectionNav, SectionNav, type SectionLink } from "@/components/shell/section-nav";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { strings } from "@/lib/strings";
import { seesTeamspaces } from "@/lib/workspace-rules";

const copy = strings.workspaces.settings;

/**
 * A workspace's settings under one heading, a tab for each part. Guests, who
 * see no member but themselves and no teamspace, get the general part alone.
 */
export function WorkspaceSettingsLayout() {
  const workspace = useWorkspace();
  const params = { slug: workspace.slug };
  const links: SectionLink[] = [
    { to: "/w/$slug/settings", params, label: copy.tabs.general },
    ...(seesTeamspaces(workspace.role)
      ? ([
          { to: "/w/$slug/settings/members", params, label: copy.tabs.members },
          { to: "/w/$slug/settings/agents", params, label: copy.tabs.agents },
          { to: "/w/$slug/settings/webhooks", params, label: copy.tabs.webhooks },
          { to: "/w/$slug/settings/teamspaces", params, label: copy.tabs.teamspaces },
        ] satisfies SectionLink[])
      : []),
  ];
  return (
    <>
      <PageHeader eyebrow={copy.eyebrow(workspace.name)} title={copy.title} lede={copy.lede} />
      <SectionNav label={copy.tabs.label} links={links} />
      <div className={afterSectionNav}>
        <Outlet />
      </div>
    </>
  );
}
