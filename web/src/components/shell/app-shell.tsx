import { Link } from "@tanstack/react-router";
import { Menu } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useInboxFeed } from "@/api/inbox-feed";
import { HueDot } from "@/components/nodes/hue-dot";
import { PageNavProvider } from "@/components/pages/page-nav-state";
import { WorkspaceAccent } from "@/components/theme/workspace-accent";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { strings } from "@/lib/strings";
import { Sidebar } from "./sidebar";

/**
 * Notion's layout: a sidebar on the left and the page on the dot grid beside
 * it. Below the md breakpoint the sidebar moves into a drawer opened from a
 * top bar, so the page keeps the full width of a phone. The accent follows
 * the workspace the sidebar shows, for whoever chose none of their own. A
 * page set to full width takes the whole column beside the sidebar. The
 * inbox's live feed stays open while it shows, for the sidebar's count.
 */
export function AppShell({ children }: { children: ReactNode }) {
  const [drawerOpen, setDrawerOpen] = useState(false);
  useInboxFeed();

  return (
    <PageNavProvider>
      <div className="min-h-dvh md:grid md:grid-cols-[15rem_minmax(0,1fr)]">
        <WorkspaceAccent />
        <a
          href="#main"
          className="fixed top-[calc(--spacing(3)+env(safe-area-inset-top,0px))] left-3 z-50 -translate-y-24 rounded-full bg-primary px-4 py-2 text-sm text-primary-foreground focus:translate-y-0"
        >
          {strings.skipToContent}
        </a>

        <header className="sticky top-0 z-30 flex items-center gap-2 border-b border-border bg-raised safe-top-1 safe-right-3 pb-1 safe-left-2 md:hidden">
          <Sheet open={drawerOpen} onOpenChange={setDrawerOpen}>
            <SheetTrigger asChild>
              <Button variant="ghost" size="icon" aria-label={strings.nav.open}>
                <Menu aria-hidden />
              </Button>
            </SheetTrigger>
            <SheetContent closeLabel={strings.nav.close}>
              <SheetTitle className="sr-only">{strings.nav.label}</SheetTitle>
              <Sidebar onNavigate={() => setDrawerOpen(false)} />
            </SheetContent>
          </Sheet>
          <Link
            to="/"
            className="flex min-h-(--tap) items-center gap-2 rounded-md px-1 text-sm font-semibold tracking-tight"
          >
            <HueDot className="size-2.5" />
            {strings.product}
          </Link>
        </header>

        <aside className="sticky top-0 hidden h-dvh overflow-y-auto border-r border-border bg-raised safe-top-4 pr-3 safe-bottom-4 safe-left-3 md:block">
          <Sidebar />
        </aside>

        <main
          id="main"
          tabIndex={-1}
          className="min-w-0 pt-8 safe-right-5 safe-bottom-12 safe-left-5 sm:safe-right-10 sm:safe-left-10 md:pt-16"
        >
          <div className="mx-auto max-w-4xl has-[[data-page-width=full]]:max-w-none">
            {children}
          </div>
        </main>
      </div>
    </PageNavProvider>
  );
}
