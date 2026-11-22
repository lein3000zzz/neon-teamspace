import { PageSection } from "@/components/settings/page-section";
import { InvitationList } from "@/components/workspaces/invitation-list";
import { strings } from "@/lib/strings";

const copy = strings.account.invitations;

/** The invitations into workspaces that wait for the user's answer. */
export function InvitationsPage() {
  return (
    <PageSection title={copy.title} hint={copy.hint}>
      <InvitationList />
    </PageSection>
  );
}
