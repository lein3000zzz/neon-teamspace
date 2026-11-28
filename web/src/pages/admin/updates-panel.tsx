import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { keys } from "@/api/keys";
import {
  PROGRESS_EVERY_MS,
  PROGRESS_FOR_MS,
  updateProgressQuery,
  updatesQuery,
  useApplyUpdate,
  useCheckUpdates,
  type Release,
  type Updates,
  type UpdaterStatus,
} from "@/api/updates";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading, sentence } from "@/components/feedback/error-notice";
import { Chip, ChipList } from "@/components/nodes/chip";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { PageSection } from "@/components/settings/page-section";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { formatDateTime } from "@/lib/format";
import { reloadPage } from "@/lib/reload";
import { strings } from "@/lib/strings";

const copy = strings.admin.updates;

/** The docs on upgrading, served beside the app. */
const UPGRADING_DOCS = "/docs/self-hosting/upgrading/";
const UPDATER_DOCS = `${UPGRADING_DOCS}#one-click-updates`;

/** How long the page says the update is done before it reloads. */
const RELOAD_AFTER_MS = 2_000;

/** Pulls the images NEON_VERSION names and recreates the services, as the updater does. */
const COMMANDS = [
  "docker compose -f deployments/docker-compose.yml pull",
  "docker compose -f deployments/docker-compose.yml up -d --wait",
].join("\n");

const link = "text-foreground underline underline-offset-4";

/**
 * The release the server runs, the newest one its last check saw on GitHub, and updating to
 * it: through the updater where one is set up, with the update followed until the server is
 * back on the new release, or else with the commands that update by hand.
 */
export function UpdatesPanel() {
  const updates = useQuery(updatesQuery);
  const check = useCheckUpdates();

  return (
    <PageSection
      title={copy.title}
      hint={copy.hint}
      actions={
        updates.data?.check === true && (
          <Button variant="outline" disabled={check.isPending} onClick={() => check.mutate()}>
            <RefreshCw aria-hidden />
            {check.isPending ? copy.checking : copy.check}
          </Button>
        )
      }
    >
      {updates.isPending && <Loading />}
      {updates.isError && (
        <ErrorNotice
          error={updates.error}
          retrying={updates.isFetching}
          onRetry={() => void updates.refetch()}
        />
      )}
      {check.isError && <ErrorNotice error={check.error} />}
      {updates.data && <Known updates={updates.data} />}
    </PageSection>
  );
}

/** What the server knows of releases, and what may be done about it. */
function Known({ updates }: { updates: Updates }) {
  const latest = updates.latest ?? undefined;
  const status = updates.updater.status ?? undefined;
  const apply = useApplyUpdate();
  const { reset } = apply;
  // The update this page started, and one under way when it opened, as after a reload.
  const following =
    (apply.isSuccess ? apply.variables : undefined) ??
    (status?.state === "updating" ? (status.version ?? undefined) : undefined);

  return (
    <div className="grid gap-4 rounded-lg border border-border bg-card p-4">
      <KeyValueList className="text-sm">
        <KeyValueRow label={copy.running}>
          <span className="font-mono">
            {updates.development ? copy.development(updates.current) : updates.current}
          </span>
        </KeyValueRow>
        {updates.check && (
          <>
            <KeyValueRow label={copy.newest}>
              {latest ? <LatestRelease release={latest} /> : copy.noneFound}
            </KeyValueRow>
            <KeyValueRow label={copy.lastChecked}>
              {updates.checked_at ? formatDateTime(updates.checked_at) : copy.notChecked}
            </KeyValueRow>
            {updates.error && (
              <KeyValueRow label={copy.checkFailed}>
                <span className="font-mono text-xs break-all">{updates.error}</span>
              </KeyValueRow>
            )}
          </>
        )}
        {updates.updater.set_up && (
          <KeyValueRow label={copy.updater}>
            <UpdaterState updater={updates.updater} />
          </KeyValueRow>
        )}
      </KeyValueList>

      {!updates.check && <p className="text-sm text-muted-foreground">{copy.off}</p>}
      {updates.check && latest && !updates.newer && (
        <p className="text-sm text-muted-foreground">
          {updates.development ? copy.developmentNote : copy.upToDate}
        </p>
      )}

      {following !== undefined ? (
        <UpdateProgress key={following} version={following} onFailed={reset} />
      ) : (
        latest &&
        updates.newer && <Newer updates={updates} latest={latest} status={status} apply={apply} />
      )}
    </div>
  );
}

/**
 * Whether the updater answers the server, and how its latest update went, so that its set-up
 * shows before any release is out; or why the server cannot ask it.
 */
function UpdaterState({ updater }: { updater: Updates["updater"] }) {
  const status = updater.status ?? undefined;
  switch (status?.state) {
    case "idle":
    case "done":
      return copy.updaterReady(status.updater_version);
    case "updating":
      return copy.updaterUpdating(status.version ?? undefined);
    case "failed":
      return copy.updaterFailed(status.version ?? undefined);
    case undefined:
      return sentence(updater.error ?? copy.updaterNoAnswer);
  }
}

/** A release's version, linked to its notes, and when it came out. */
function LatestRelease({ release }: { release: Release }) {
  return (
    <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
      <a
        href={release.url}
        target="_blank"
        rel="noreferrer"
        aria-label={copy.notesLabel(release.version)}
        className={`font-mono ${link}`}
      >
        {release.version}
      </a>
      {release.prerelease && (
        <ChipList>
          <Chip>{copy.prerelease}</Chip>
        </ChipList>
      )}
      {release.published_at && (
        <span className="text-muted-foreground">
          {copy.published(formatDateTime(release.published_at))}
        </span>
      )}
    </span>
  );
}

type NewerProps = {
  updates: Updates;
  latest: Release;
  status: UpdaterStatus | undefined;
  apply: ReturnType<typeof useApplyUpdate>;
};

/** A newer release: the button that has the updater update to it, or the commands to. */
function Newer({ updates, latest, status, apply }: NewerProps) {
  const [asking, setAsking] = useState(false);
  const failed = status?.state === "failed" ? status : undefined;

  return (
    <Alert live="polite">
      <AlertTitle>{copy.newer(latest.version)}</AlertTitle>
      <AlertDescription>
        <p>
          <a href={latest.url} target="_blank" rel="noreferrer" className={link}>
            {copy.notes}
          </a>
          . {copy.notesBefore}
        </p>
        {updates.updater.set_up ? (
          <>
            <p>{copy.updaterNote}</p>
            {failed && <FailedUpdate status={failed} />}
            <div>
              <Button disabled={apply.isPending} onClick={() => setAsking(true)}>
                {copy.update(latest.version)}
              </Button>
            </div>
            {apply.isError && <ErrorNotice error={apply.error} />}
            <ConfirmDialog
              open={asking}
              title={copy.confirmTitle(latest.version)}
              description={copy.confirmLede}
              confirm={copy.confirm}
              onCancel={() => setAsking(false)}
              onConfirm={() => {
                setAsking(false);
                apply.mutate(latest.version);
              }}
            />
          </>
        ) : (
          <ByHand version={latest.version} />
        )}
      </AlertDescription>
    </Alert>
  );
}

/** The updater's own account of an update that failed: why, and the end of its log. */
function FailedUpdate({ status }: { status: UpdaterStatus }) {
  return (
    <div className="grid gap-2">
      <p className="text-foreground">
        {copy.lastFailed(status.version ?? "", status.error ?? undefined)}
      </p>
      {status.log && (
        <details>
          <summary className="cursor-pointer">{copy.log}</summary>
          <UpdaterLog log={status.log} />
        </details>
      )}
    </div>
  );
}

/** The commands that update by hand, which the updater would otherwise run. */
function ByHand({ version }: { version: string }) {
  return (
    <>
      <p>{copy.byHand(version)}</p>
      <CommandBlock label={copy.envFile} text={`NEON_VERSION=${version}`} />
      <p>{copy.thenRun}</p>
      <CommandBlock label={copy.commands} text={COMMANDS} />
      <p>
        {copy.byHandDocs}{" "}
        <a href={UPGRADING_DOCS} target="_blank" rel="noreferrer" className={link}>
          {copy.upgradingDocs}
        </a>
        .{" "}
        <a href={UPDATER_DOCS} target="_blank" rel="noreferrer" className={link}>
          {copy.updaterDocs}
        </a>{" "}
        {copy.updaterDocsAfter}
      </p>
    </>
  );
}

/**
 * Text to copy, kept as it is written. Where the browser has no clipboard, or refuses it,
 * the text is selected for copying by hand.
 */
function CommandBlock({ label, text }: { label: string; text: string }) {
  const block = useRef<HTMLPreElement>(null);
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      const pre = block.current;
      if (pre !== null) {
        window.getSelection()?.selectAllChildren(pre);
      }
      setState("failed");
    }
  };

  return (
    <div className="grid gap-1">
      <div className="flex items-start gap-2">
        {/* Named on a figure, as a pre may carry no name. */}
        <figure aria-label={label} className="min-w-0 flex-1">
          <pre
            ref={block}
            className="overflow-x-auto rounded-md border border-border bg-background p-2 font-mono text-xs text-foreground"
          >
            {text}
          </pre>
        </figure>
        <Button type="button" variant="outline" size="sm" onClick={() => void onCopy()}>
          {state === "copied" ? <Check aria-hidden /> : <Copy aria-hidden />}
          {state === "copied" ? strings.copy.copied : strings.copy.copy}{" "}
          <span className="sr-only">{label}</span>
        </Button>
      </div>
      <output className="block text-xs empty:hidden">
        {state === "failed" ? strings.copy.failed : ""}
      </output>
    </div>
  );
}

type UpdateProgressProps = {
  version: string;
  /** The update failed: the panel goes back to offering it, with the updater's account. */
  onFailed: () => void;
};

/**
 * An update under way, asked after every few seconds through the restart, until the server
 * runs the new release, when the page reloads for its app; or until the updater says it
 * failed, or that it is done while the server runs what it ran, or ten minutes pass.
 */
function UpdateProgress({ version, onFailed }: UpdateProgressProps) {
  const queryClient = useQueryClient();
  const [timedOut, setTimedOut] = useState(false);
  const [lastLog, setLastLog] = useState<string>();
  const progress = useQuery({
    ...updateProgressQuery(version),
    refetchInterval: (query) => {
      const phase = query.state.data?.phase;
      return timedOut || phase === "done" || phase === "failed" || phase === "unchanged"
        ? false
        : PROGRESS_EVERY_MS;
    },
    refetchIntervalInBackground: true,
  });
  const phase = progress.data?.phase;
  const log = progress.data?.phase === "updating" ? progress.data.status?.log : undefined;

  useEffect(() => {
    const timer = setTimeout(() => setTimedOut(true), PROGRESS_FOR_MS);
    return () => clearTimeout(timer);
  }, []);

  // The log stays in sight while the server restarts and its updater cannot be asked.
  if (log && log !== lastLog) {
    setLastLog(log);
  }

  useEffect(() => {
    if (phase === "done") {
      const timer = setTimeout(reloadPage, RELOAD_AFTER_MS);
      return () => clearTimeout(timer);
    }
    if (phase === "failed") {
      void queryClient.invalidateQueries({ queryKey: keys.updates }).then(onFailed);
    }
    return undefined;
  }, [phase, queryClient, onFailed]);

  // The log is never inside a live region, which would have it read out whole each time it
  // changes: the short sentences beside it say where the update stands.
  if (progress.data?.phase === "unchanged") {
    const { status, running } = progress.data;
    return (
      <div className="grid gap-2">
        <Alert tone="error" live="assertive">
          <AlertTitle>{copy.updating(version)}</AlertTitle>
          <AlertDescription>
            <p className="text-foreground">{copy.unchanged(version, running)}</p>
            <p>
              {copy.unchangedWhy}{" "}
              <a href={UPDATER_DOCS} target="_blank" rel="noreferrer" className={link}>
                {copy.unchangedDocs}
              </a>
              .
            </p>
          </AlertDescription>
        </Alert>
        {status.log && <UpdaterLog log={status.log} />}
      </div>
    );
  }

  if (timedOut && phase !== "done") {
    return (
      <div className="grid gap-2">
        <Alert tone="error" live="assertive">
          <AlertTitle>{copy.updating(version)}</AlertTitle>
          <AlertDescription>
            <p className="text-foreground">{copy.timedOut(version)}</p>
          </AlertDescription>
        </Alert>
        {lastLog && <UpdaterLog log={lastLog} />}
      </div>
    );
  }

  return (
    <Alert live="off">
      <AlertTitle>{copy.updating(version)}</AlertTitle>
      <AlertDescription>
        <p>{copy.updatingLede}</p>
        <output className="block text-foreground">
          {phase === "done"
            ? copy.phases.done(version)
            : phase === "restarting"
              ? copy.phases.restarting
              : copy.phases.updating}
        </output>
        {lastLog && phase !== "done" && <UpdaterLog log={lastLog} />}
      </AlertDescription>
    </Alert>
  );
}

/**
 * The end of the updater's log, kept scrolled to its last line as it grows. It is named, and
 * silent: no log role, as each poll rewrites the whole text, which a log would read again.
 */
function UpdaterLog({ log }: { log: string }) {
  const block = useRef<HTMLPreElement>(null);
  useEffect(() => {
    const pre = block.current;
    if (pre !== null && log !== "") {
      pre.scrollTop = pre.scrollHeight;
    }
  }, [log]);

  return (
    <figure aria-label={copy.log}>
      <pre
        ref={block}
        className="max-h-64 overflow-auto rounded-md border border-border bg-background p-2 font-mono text-xs whitespace-pre-wrap text-foreground"
      >
        {log}
      </pre>
    </figure>
  );
}
