import type { CreatedInvite } from "@/api/admin";
import { CopyField } from "@/components/feedback/copy-field";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { formatDateTime } from "@/lib/format";
import { inviteLink } from "@/lib/links";
import { strings } from "@/lib/strings";

const copy = strings.admin.invites;

/**
 * A new invite's link and code, shown now; later only from its sealed copy,
 * when the server keeps one. A mailed invite whose mail did not go says so,
 * so the admin can hand the link over another way.
 */
export function CreatedInviteNotice({
  created,
  onDone,
}: {
  created: CreatedInvite;
  onDone: () => void;
}) {
  const { invite, code, sent } = created;
  const bound = invite.email ?? undefined;
  const failed = bound !== undefined && sent === false;
  const title =
    bound === undefined
      ? copy.madeTitle
      : failed
        ? copy.notMailedTitle(bound)
        : copy.mailedTitle(bound);
  return (
    <Alert live="polite" tone={failed ? "error" : "info"}>
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription>
        <p>
          {copy.admits(invite.max_uses, formatDateTime(invite.expires_at))}
          {bound !== undefined && ` ${copy.boundTo(bound)}`}
        </p>
        <CopyField label={copy.link} value={inviteLink(code)} />
        <CopyField label={copy.codeLabel} value={code} />
        {!invite.code_kept && <p>{copy.shownOnce}</p>}
        <div>
          <Button variant="outline" size="sm" onClick={onDone}>
            {copy.done}
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  );
}
