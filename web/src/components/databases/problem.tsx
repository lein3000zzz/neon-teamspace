import { X } from "lucide-react";
import { hasCode } from "@/api/errors";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";

const copy = strings.databases.problem;

/**
 * What a refused change comes to, in a few words over the server's own
 * message: a database full of rows, a limit passed, such as a row's values
 * growing past 64 KiB, or a change the user may not make.
 */
export function problemTitle(error: unknown): string {
  if (hasCode(error, "too_many_rows")) {
    return copy.tooManyRows;
  }
  if (hasCode(error, "limit_reached")) {
    return copy.limit;
  }
  return copy.notSaved;
}

type ProblemProps = {
  error: unknown;
  onDismiss: () => void;
};

/**
 * Why a change made in a database was not saved, until it is dismissed: the
 * value it showed at once is put back as the rows are read again.
 */
export function Problem({ error, onDismiss }: ProblemProps) {
  return (
    <div className="relative">
      <ErrorNotice error={error} title={problemTitle(error)}>
        {hasCode(error, "too_many_rows") && <p>{copy.tooManyRowsHint}</p>}
      </ErrorNotice>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={copy.dismiss}
        onClick={onDismiss}
        className="absolute top-1.5 right-1.5"
      >
        <X aria-hidden />
      </Button>
    </div>
  );
}
