import { useQuery } from "@tanstack/react-query";
import { Send } from "lucide-react";
import { useState } from "react";
import { settingsQuery, useSendTestMail, type MailStatus } from "@/api/admin";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Chip, ChipList } from "@/components/nodes/chip";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { Button } from "@/components/ui/button";
import { formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";

const copy = strings.admin.settings.mail;

/** How long the panel keeps asking how a test mail went, and how often. */
const WATCH_FOR_MS = 60 * 1000;
const WATCH_EVERY_MS = 2 * 1000;

/** The last delivery and the last failure, to tell when either changes. */
function outcome(mail: MailStatus): string {
  return `${mail.last_sent_at ?? ""} ${mail.last_failure?.at ?? ""}`;
}

/**
 * Whether mail can go, how the mail server took the last mail since the
 * server started, and a test mail to the admin's own address. Mail goes
 * through the job queue, so after a test the settings are read again every
 * few seconds until the status changes, for a minute at most.
 */
export function MailStatusPanel({ mail }: { mail: MailStatus }) {
  const test = useSendTestMail();
  const [watch, setWatch] = useState<{ from: string; until: number }>();
  useQuery({
    ...settingsQuery,
    enabled: watch !== undefined,
    refetchInterval: (query) => {
      const latest = query.state.data?.mail;
      return watch !== undefined &&
        latest !== undefined &&
        outcome(latest) === watch.from &&
        Date.now() < watch.until
        ? WATCH_EVERY_MS
        : false;
    },
  });

  const failure = mail.last_failure ?? undefined;
  const sendTest = () => {
    const from = outcome(mail);
    test.mutate(undefined, {
      onSuccess: () => setWatch({ from, until: Date.now() + WATCH_FOR_MS }),
    });
  };

  return (
    <div className="grid gap-3 rounded-lg border border-border bg-card p-4">
      <div className="flex flex-wrap items-center gap-3">
        <h3 className="text-sm font-semibold">{copy.title}</h3>
        <ChipList>
          <Chip tone={mail.configured ? "strong" : "default"}>
            {mail.configured ? copy.ready : copy.notReady}
          </Chip>
        </ChipList>
      </div>
      {!mail.configured && mail.problem && (
        <p className="text-sm text-muted-foreground">{copy.missing(mail.problem)}</p>
      )}
      <KeyValueList className="text-sm">
        <KeyValueRow label={copy.lastSent}>
          {mail.last_sent_at ? formatDateTime(mail.last_sent_at) : copy.noneSent}
        </KeyValueRow>
        <KeyValueRow label={copy.lastFailure}>
          {failure === undefined ? (
            copy.noFailure
          ) : (
            <span className="grid gap-1">
              <span>
                {copy.failedAt(
                  copy.kinds[failure.kind] ?? failure.kind,
                  formatDateTime(failure.at),
                )}
              </span>
              <span className="font-mono text-xs break-all">{failure.error}</span>
              <span className="text-muted-foreground">
                {failure.permanent ? copy.givenUp : copy.retried}
              </span>
            </span>
          )}
        </KeyValueRow>
      </KeyValueList>
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="outline" disabled={!mail.configured || test.isPending} onClick={sendTest}>
          <Send aria-hidden />
          {test.isPending ? copy.testPending : copy.test}
        </Button>
        <output className="block text-sm text-muted-foreground empty:hidden">
          {test.isSuccess ? copy.testQueued(test.data) : ""}
        </output>
      </div>
      {test.isError && <ErrorNotice error={test.error} />}
    </div>
  );
}
