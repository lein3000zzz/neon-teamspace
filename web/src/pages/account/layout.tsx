import { Outlet } from "@tanstack/react-router";
import { PageHeader } from "@/components/shell/page-header";
import { afterSectionNav, SectionNav } from "@/components/shell/section-nav";
import { strings } from "@/lib/strings";

const copy = strings.account;

/** The account pages under one heading, with a tab for each. */
export function AccountLayout() {
  return (
    <>
      <PageHeader eyebrow={copy.eyebrow} title={copy.title} lede={copy.lede} />
      <SectionNav
        label={copy.tabs.label}
        links={[
          { to: "/account", label: copy.tabs.profile },
          { to: "/account/invitations", label: copy.tabs.invitations },
          { to: "/account/sessions", label: copy.tabs.sessions },
          { to: "/account/tokens", label: copy.tabs.tokens },
          { to: "/account/apps", label: copy.tabs.apps },
          { to: "/account/appearance", label: copy.tabs.appearance },
        ]}
      />
      <div className={afterSectionNav}>
        <Outlet />
      </div>
    </>
  );
}
