import { UserPlus, X } from "lucide-react";
import { useState } from "react";
import type { Page } from "@/api/pages";
import type { Person } from "@/api/workspaces";
import { ShareDialog } from "@/components/pages/share-dialog";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { mayShare } from "@/lib/page-rules";
import { attribution } from "@/lib/people";
import { strings } from "@/lib/strings";

const copy = strings.comments;

const names = new Intl.ListFormat("en", { type: "conjunction" });

type UnseenNoticeProps = {
  /** Those mentioned who may not see the page, and so were told nothing. */
  people: readonly Person[];
  page: Page;
  onDismiss: () => void;
};

/**
 * Says that a mention told someone nothing, as they may not see the page, and
 * offers to share the page with them, the first of them chosen, to whoever
 * may share it.
 */
export function UnseenNotice({ people, page, onDismiss }: UnseenNoticeProps) {
  const workspace = useWorkspace();
  const [sharing, setSharing] = useState(false);
  const first = people[0];
  return (
    <Alert live="polite">
      <div className="grid gap-2">
        <div className="flex items-start gap-2">
          <p className="min-w-0 flex-1 text-foreground">
            {copy.unseen(names.format(people.map(attribution)))}
          </p>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={copy.dismiss}
            onClick={onDismiss}
            className="-my-1"
          >
            <X aria-hidden />
          </Button>
        </div>
        {mayShare(workspace.role, page.access) && (
          <Button
            variant="outline"
            size="sm"
            className="justify-self-start"
            onClick={() => setSharing(true)}
          >
            <UserPlus aria-hidden />
            {copy.share}
          </Button>
        )}
      </div>
      {sharing && (
        <ShareDialog
          workspace={workspace}
          page={page}
          open
          onOpenChange={setSharing}
          {...(first === undefined ? {} : { suggested: first.id })}
        />
      )}
    </Alert>
  );
}
