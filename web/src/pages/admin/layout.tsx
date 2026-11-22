import { Outlet } from "@tanstack/react-router";
import { PageHeader } from "@/components/shell/page-header";
import { afterSectionNav, SectionNav } from "@/components/shell/section-nav";
import { useMe } from "@/components/shell/signed-in";
import { strings } from "@/lib/strings";
import { NotAllowedPage } from "@/pages/not-allowed";

const copy = strings.admin;

/**
 * The admin pages, for admins. Anyone else gets the not-allowed page and none
 * of the admin requests go out; the server would refuse them in any case.
 */
export function AdminLayout() {
  const me = useMe();
  if (me.role !== "admin") {
    return <NotAllowedPage />;
  }
  return (
    <>
      <PageHeader eyebrow={copy.eyebrow} title={copy.title} lede={copy.lede} />
      <SectionNav
        label={copy.tabs.label}
        links={[
          { to: "/admin", label: copy.tabs.users },
          { to: "/admin/invites", label: copy.tabs.invites },
          { to: "/admin/requests", label: copy.tabs.requests },
          { to: "/admin/settings", label: copy.tabs.settings },
        ]}
      />
      <div className={afterSectionNav}>
        <Outlet />
      </div>
    </>
  );
}
