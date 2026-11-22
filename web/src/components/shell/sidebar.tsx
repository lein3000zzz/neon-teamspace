import { Link } from "@tanstack/react-router";
import { House, LogOut, Shapes, ShieldCheck, UserRound } from "lucide-react";
import { useId } from "react";
import { useLogout } from "@/api/auth";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { HueDot } from "@/components/nodes/hue-dot";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { strings } from "@/lib/strings";
import { InstanceStatus } from "./instance-status";
import { useMe } from "./signed-in";

// The router marks the current link with aria-current="page"; the accent bar
// on its left edge is the active item border, which glows in dark mode.
const navLink =
  "relative flex min-h-(--tap) items-center gap-2.5 rounded-md px-2.5 text-sm text-muted-foreground transition-colors before:absolute before:inset-y-1.5 before:left-0 before:w-0.5 before:rounded-full before:bg-primary before:opacity-0 before:shadow-glow hover:bg-accent hover:text-foreground aria-[current=page]:bg-accent aria-[current=page]:text-foreground aria-[current=page]:before:opacity-100";

type SidebarProps = {
  /** Called after a link is followed, so the drawer can close itself. */
  onNavigate?: () => void;
};

/**
 * Brand, navigation, who is signed in, and the instance footer. Rendered as a
 * column beside the page on wide screens and inside the drawer on narrow
 * ones. The admin pages are listed for admins only; the server refuses them
 * to anyone else whatever the menu shows.
 */
export function Sidebar({ onNavigate }: SidebarProps) {
  // The column and the open drawer are mounted together, so ids must be unique per copy.
  const settingsId = useId();
  const me = useMe();
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

      <nav aria-label={strings.nav.label} className="grid gap-5">
        <ul className="grid gap-0.5">
          <li>
            <Link to="/" activeOptions={{ exact: true }} onClick={onNavigate} className={navLink}>
              <House aria-hidden className="size-4" />
              {strings.nav.home}
            </Link>
          </li>
          <li>
            <Link to="/styleguide" onClick={onNavigate} className={navLink}>
              <Shapes aria-hidden className="size-4" />
              {strings.nav.styleguide}
            </Link>
          </li>
        </ul>
        <div className="grid gap-1">
          <p
            id={settingsId}
            className="px-2.5 font-mono text-[0.68rem] tracking-widest text-muted-foreground uppercase"
          >
            {strings.nav.settings}
          </p>
          <ul aria-labelledby={settingsId} className="grid gap-0.5">
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
