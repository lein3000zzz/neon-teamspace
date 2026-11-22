import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  House,
  Inbox,
  LogOut,
  Settings,
  Shapes,
  ShieldCheck,
  Trash2,
  UserRound,
} from "lucide-react";
import { useId } from "react";
import { useLogout } from "@/api/auth";
import { myInvitationsQuery } from "@/api/workspaces";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { HueDot } from "@/components/nodes/hue-dot";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { useCurrentWorkspace } from "@/components/workspaces/workspace-context";
import { hasPrivatePages } from "@/lib/page-rules";
import { strings } from "@/lib/strings";
import { seesTeamspaces } from "@/lib/workspace-rules";
import { InstanceStatus } from "./instance-status";
import { navHeading, navLink } from "./nav-styles";
import { FavoritesNav, PageNavNotice, PrivateNav, SharedNav } from "./page-nav";
import { useMe } from "./signed-in";
import { TeamspaceNav } from "./teamspace-nav";
import { WorkspaceSwitcher } from "./workspace-switcher";

type SidebarProps = {
  /** Called after a link is followed, so the drawer can close itself. */
  onNavigate?: () => void;
};

/**
 * Brand, the workspace switcher, navigation, who is signed in, and the
 * instance footer. Rendered as a column beside the page on wide screens and
 * inside the drawer on narrow ones. The admin pages are listed for admins
 * only; the server refuses them to anyone else whatever the menu shows.
 */
export function Sidebar({ onNavigate }: SidebarProps) {
  // The column and the open drawer are mounted together, so ids must be unique per copy.
  const settingsId = useId();
  const me = useMe();
  const { current } = useCurrentWorkspace();
  const invitations = useQuery(myInvitationsQuery);
  const waiting = invitations.data?.length ?? 0;

  return (
    <div className="flex min-h-full flex-col gap-6">
      <Link
        to="/"
        onClick={onNavigate}
        className="flex min-h-(--tap) items-center gap-2.5 self-start rounded-md px-2.5 text-sm font-semibold tracking-tight"
      >
        <HueDot className="size-2.5" />
        {strings.product}
      </Link>

      <WorkspaceSwitcher onNavigate={onNavigate} />

      <nav aria-label={strings.nav.label} className="grid gap-5">
        <ul className="grid gap-0.5">
          <li>
            {current === undefined ? (
              <Link to="/" activeOptions={{ exact: true }} onClick={onNavigate} className={navLink}>
                <House aria-hidden className="size-4" />
                {strings.nav.home}
              </Link>
            ) : (
              <Link
                to="/w/$slug"
                params={{ slug: current.slug }}
                activeOptions={{ exact: true }}
                onClick={onNavigate}
                className={navLink}
              >
                <House aria-hidden className="size-4" />
                {strings.nav.home}
              </Link>
            )}
          </li>
          {waiting > 0 && (
            <li>
              <Link
                to="/account/invitations"
                onClick={onNavigate}
                aria-label={strings.nav.pending(waiting)}
                className={navLink}
              >
                <Inbox aria-hidden className="size-4" />
                <span className="flex-1">{strings.nav.invitations}</span>
                <span
                  aria-hidden
                  className="grid h-5 min-w-5 place-items-center rounded-full border border-primary px-1.5 font-mono text-[0.68rem] text-primary tabular-nums"
                >
                  {waiting}
                </span>
              </Link>
            </li>
          )}
          {current !== undefined && (
            <li>
              <Link
                to="/w/$slug/trash"
                params={{ slug: current.slug }}
                onClick={onNavigate}
                className={navLink}
              >
                <Trash2 aria-hidden className="size-4" />
                {strings.nav.trash}
              </Link>
            </li>
          )}
          <li>
            <Link to="/styleguide" onClick={onNavigate} className={navLink}>
              <Shapes aria-hidden className="size-4" />
              {strings.nav.styleguide}
            </Link>
          </li>
        </ul>
        {current !== undefined && (
          <>
            <PageNavNotice workspace={current} />
            <FavoritesNav workspace={current} onNavigate={onNavigate} />
            {seesTeamspaces(current.role) && (
              <TeamspaceNav workspace={current} onNavigate={onNavigate} />
            )}
            {hasPrivatePages(current.role) && (
              <PrivateNav workspace={current} onNavigate={onNavigate} />
            )}
            <SharedNav workspace={current} onNavigate={onNavigate} />
          </>
        )}
        <div className="grid gap-1">
          <p id={settingsId} className={navHeading}>
            {strings.nav.settings}
          </p>
          <ul aria-labelledby={settingsId} className="grid gap-0.5">
            {current !== undefined && (
              <li>
                <Link
                  to="/w/$slug/settings"
                  params={{ slug: current.slug }}
                  onClick={onNavigate}
                  className={navLink}
                >
                  <Settings aria-hidden className="size-4" />
                  {strings.nav.workspaceSettings}
                </Link>
              </li>
            )}
            <li>
              <Link to="/account" onClick={onNavigate} className={navLink}>
                <UserRound aria-hidden className="size-4" />
                {strings.nav.account}
              </Link>
            </li>
            {me.role === "admin" && (
              <li>
                <Link to="/admin" onClick={onNavigate} className={navLink}>
                  <ShieldCheck aria-hidden className="size-4" />
                  {strings.nav.admin}
                </Link>
              </li>
            )}
          </ul>
        </div>
      </nav>

      <div className="mt-auto grid gap-3">
        <Separator />
        <SignedInAs />
        <Separator />
        <div className="px-2.5">
          <InstanceStatus />
        </div>
      </div>
    </div>
  );
}

/** Who is signed in, and the way out. */
function SignedInAs() {
  const me = useMe();
  const logout = useLogout();
  return (
    <section aria-label={strings.nav.signedIn} className="grid gap-2 px-2.5">
      <div className="grid min-w-0 text-xs">
        <span className="truncate font-medium text-foreground">{me.display_name}</span>
        <span className="truncate text-muted-foreground">{me.email}</span>
      </div>
      {logout.isError && <ErrorNotice error={logout.error} />}
      <Button
        variant="outline"
        size="sm"
        className="justify-self-start"
        disabled={logout.isPending}
        onClick={() => logout.mutate()}
      >
        <LogOut aria-hidden />
        {logout.isPending ? strings.nav.signingOut : strings.nav.signOut}
      </Button>
    </section>
  );
}
