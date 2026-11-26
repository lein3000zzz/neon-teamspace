import { useInfiniteQuery, useQuery, type UseQueryResult } from "@tanstack/react-query";
import { cn } from "cn";
import { History } from "lucide-react";
import { lazy, Suspense, useId, useRef, useState, type KeyboardEvent } from "react";
import { hasCode } from "@/api/errors";
import {
  comparisonQuery,
  useRestoreVersion,
  versionQuery,
  versionsQuery,
  type BlockChange,
  type Comparison,
  type Version,
} from "@/api/history";
import type { Page } from "@/api/pages";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { PersonLabel } from "@/components/people/person-label";
import { useWorkspacePeople } from "@/components/people/use-people";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";
import { seesTeamspaces } from "@/lib/workspace-rules";

const VersionView = lazy(() => import("@/components/editor/version-view"));

const copy = strings.history;

type HistoryButtonProps = {
  page: Page;
  /** The user edits the page, out of the trash, and so may restore a version of it. */
  editable: boolean;
};

/**
 * The page's history, in a panel over its side, opened from the page's header:
 * for those who edit the page, as a version may hold what was taken out of it
 * before someone was given it to view.
 */
export function HistoryButton({ page, editable }: HistoryButtonProps) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="ghost" size="sm" onClick={() => setOpen(true)}>
        <History aria-hidden />
        {copy.open}
      </Button>
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent side="right" closeLabel={copy.close}>
          <SheetTitle className="pt-1 pr-12 text-lg font-semibold">{copy.title}</SheetTitle>
          {open && <HistoryPanel page={page} editable={editable} />}
        </SheetContent>
      </Sheet>
    </>
  );
}

/**
 * The versions of a page, latest first, each with when, who changed the page
 * since the one before, and how much; the one picked compared with the page
 * as it is now, or shown as it was, and restored after a confirmation.
 */
function HistoryPanel({ page, editable }: HistoryButtonProps) {
  const workspace = useWorkspace();
  const versions = useInfiniteQuery(versionsQuery(workspace.id, page.id));
  const list = versions.data?.pages.flatMap((part) => part.versions) ?? [];
  const [picked, setPicked] = useState<string | null>(null);
  const chosen = list.find((version) => version.id === picked) ?? list[0];

  if (versions.isPending) {
    return <Loading label={copy.loading} />;
  }
  if (versions.isError) {
    return (
      <ErrorNotice
        error={versions.error}
        retrying={versions.isFetching}
        onRetry={() => void versions.refetch()}
      />
    );
  }
  if (chosen === undefined) {
    return <p className="mt-4 pr-3 text-sm text-muted-foreground">{copy.empty}</p>;
  }
  return (
    <div className="mt-4 grid content-start gap-6 pr-3 md:grid-cols-[14rem_minmax(0,1fr)]">
      <section aria-label={copy.versions} className="grid content-start gap-2">
        <ul className="grid gap-1">
          {list.map((version) => (
            <li key={version.id}>
              <VersionItem
                version={version}
                current={version.id === chosen.id}
                onPick={() => setPicked(version.id)}
              />
            </li>
          ))}
        </ul>
        {versions.hasNextPage && (
          <Button
            variant="outline"
            size="sm"
            className="justify-self-start"
            disabled={versions.isFetchingNextPage}
            onClick={() => void versions.fetchNextPage()}
          >
            {copy.more}
          </Button>
        )}
      </section>
      <VersionDetail
        key={chosen.id}
        page={page}
        version={chosen}
        editable={editable}
        onPin={() => setPicked(chosen.id)}
      />
    </div>
  );
}

const summaryOf = (version: Pick<Version, "added" | "changed" | "removed">) =>
  copy.summary(version.added, version.changed, version.removed);

function VersionItem({
  version,
  current,
  onPick,
}: {
  version: Version;
  current: boolean;
  onPick: () => void;
}) {
  const workspace = useWorkspace();
  const people = useWorkspacePeople(workspace.id, seesTeamspaces(workspace.role));
  return (
    <button
      type="button"
      aria-pressed={current}
      onClick={onPick}
      className={cn(
        "grid w-full gap-0.5 rounded-md px-2.5 py-2 text-left text-sm hover:bg-accent",
        current && "bg-accent",
      )}
    >
      <span className="font-medium">{formatDateTime(version.created_at)}</span>
      <span className="text-xs text-muted-foreground">{summaryOf(version)}</span>
      {version.restored_from_at && (
        <span className="text-xs text-muted-foreground">
          {copy.restoredFrom(formatDateTime(version.restored_from_at))}
        </span>
      )}
      {version.authors.length > 0 && (
        <span className="flex flex-wrap gap-x-2 text-xs text-muted-foreground">
          {version.authors.map((author) => (
            <PersonLabel key={author} person={people.get(author)} />
          ))}
        </span>
      )}
    </button>
  );
}

const TABS = ["changes", "preview"] as const;
type Tab = (typeof TABS)[number];

type VersionDetailProps = HistoryButtonProps & {
  version: Version;
  /**
   * Keeps this version picked, as a restore adds versions above it, which would
   * otherwise take the place of the latest shown by default.
   */
  onPin: () => void;
};

function VersionDetail({ page, version, editable, onPin }: VersionDetailProps) {
  const workspace = useWorkspace();
  const [tab, setTab] = useState<Tab>("changes");
  const tabsId = useId();
  const panelId = `${tabsId}-panel`;
  const tabId = (each: Tab) => `${tabsId}-${each}`;
  const tabs = useRef(new Map<Tab, HTMLButtonElement>());
  const [confirming, setConfirming] = useState(false);
  const [restored, setRestored] = useState(false);
  const comparison = useQuery(comparisonQuery(workspace.id, page.id, version.id));
  const restore = useRestoreVersion(workspace.id, page.id);
  const when = formatDateTime(version.created_at);
  // A page as the version has it has nothing to restore.
  const differs =
    comparison.data !== undefined &&
    comparison.data.added + comparison.data.changed + comparison.data.removed > 0;

  // The tab pattern: the arrow keys, Home and End move between the tabs, each picked as
  // it is reached, and Tab goes from the one picked to its panel.
  const onTabKey = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    const target =
      event.key === "Home"
        ? TABS[0]
        : event.key === "End"
          ? TABS.at(-1)
          : step === 0
            ? undefined
            : TABS[(index + step + TABS.length) % TABS.length];
    if (target !== undefined) {
      event.preventDefault();
      setTab(target);
      tabs.current.get(target)?.focus();
    }
  };

  const confirm = () => {
    setConfirming(false);
    const base = comparison.data?.revision;
    if (base === undefined) {
      return;
    }
    setRestored(false);
    onPin();
    restore.mutate(
      { versionId: version.id, base },
      {
        onSuccess: () => {
          setRestored(true);
          void comparison.refetch();
        },
        // Changed since: the comparison is read again, to look at before trying again.
        onError: (error) => {
          if (hasCode(error, "edit_conflict")) {
            void comparison.refetch();
          }
        },
      },
    );
  };

  return (
    <section aria-label={when} className="grid min-w-0 content-start gap-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="font-semibold">{when}</h3>
        {editable && (
          <Button
            size="sm"
            disabled={!differs || restore.isPending}
            onClick={() => setConfirming(true)}
          >
            {restore.isPending ? copy.restoring : copy.restore}
          </Button>
        )}
      </div>
      {!editable && <p className="text-sm text-muted-foreground">{copy.inTrash}</p>}
      {restored && (
        <Alert tone="info" live="polite">
          <p>{copy.restored}</p>
        </Alert>
      )}
      {restore.isError &&
        (hasCode(restore.error, "edit_conflict") ? (
          <Alert tone="error" live="assertive">
            <p>{copy.conflict}</p>
          </Alert>
        ) : (
          <ErrorNotice error={restore.error} />
        ))}

      <div role="tablist" aria-label={copy.tabsLabel} className="flex gap-1">
        {TABS.map((each, index) => (
          <Button
            key={each}
            ref={(element) => {
              if (element === null) {
                tabs.current.delete(each);
              } else {
                tabs.current.set(each, element);
              }
            }}
            id={tabId(each)}
            role="tab"
            aria-selected={tab === each}
            aria-controls={panelId}
            tabIndex={tab === each ? 0 : -1}
            variant={tab === each ? "outline" : "ghost"}
            size="sm"
            onClick={() => setTab(each)}
            onKeyDown={(event) => onTabKey(event, index)}
          >
            {copy.tabs[each]}
          </Button>
        ))}
      </div>
      <div role="tabpanel" id={panelId} aria-labelledby={tabId(tab)} tabIndex={0}>
        {tab === "changes" ? (
          <Changes comparison={comparison} />
        ) : (
          <Preview pageId={page.id} versionId={version.id} />
        )}
      </div>

      <ConfirmDialog
        open={confirming}
        title={copy.confirmTitle}
        description={copy.confirmBody(when)}
        confirm={copy.confirm}
        onConfirm={confirm}
        onCancel={() => setConfirming(false)}
      />
    </section>
  );
}

function Changes({ comparison }: { comparison: UseQueryResult<Comparison> }) {
  if (comparison.isPending) {
    return <Loading />;
  }
  if (comparison.isError) {
    return (
      <ErrorNotice
        error={comparison.error}
        retrying={comparison.isFetching}
        onRetry={() => void comparison.refetch()}
      />
    );
  }
  const data = comparison.data;
  const changed = data.added + data.changed + data.removed > 0;
  return (
    <div className="grid gap-3">
      <p className="text-sm text-muted-foreground">
        {changed ? copy.sinceThen(summaryOf(data)) : copy.sameAsNow}
      </p>
      <ol aria-label={copy.changesLabel} className="grid gap-1">
        {keyed(data.changes).map(({ change, key }) => (
          <ChangeLine key={key} change={change} />
        ))}
      </ol>
    </div>
  );
}

/**
 * Each change with a key of its own: its block's id, which a block moved
 * elsewhere has twice, once removed and once added, or for a block without
 * one, what it holds, counted when it repeats.
 */
function keyed(changes: BlockChange[]): { change: BlockChange; key: string }[] {
  const seen = new Map<string, number>();
  return changes.map((change) => {
    const base = `${change.change}:${change.block_id ?? change.after ?? change.before ?? ""}`;
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    return { change, key: `${base}:${count}` };
  });
}

const lineStyles: Record<BlockChange["change"], string> = {
  same: "border-transparent text-muted-foreground",
  changed: "border-primary bg-primary/5",
  added: "border-ok bg-ok/10",
  removed: "border-bad bg-bad/10",
};

function ChangeLine({ change }: { change: BlockChange }) {
  const before = change.before?.trim() ?? "";
  const after = change.after?.trim() ?? "";
  return (
    <li
      className={cn(
        "grid gap-1 rounded-sm border-l-2 px-3 py-1.5 text-sm break-words whitespace-pre-wrap",
        lineStyles[change.change],
      )}
      style={{ marginLeft: `${change.depth * 1.25}rem` }}
    >
      <span className="font-mono text-[0.68rem] tracking-wider text-muted-foreground uppercase">
        {copy.change[change.change]}
      </span>
      {change.change === "changed" && (
        <>
          <del aria-label={copy.before}>{before}</del>
          <ins aria-label={copy.after} className="no-underline">
            {after}
          </ins>
        </>
      )}
      {change.change === "removed" && <del>{before}</del>}
      {change.change === "added" && <ins className="no-underline">{after}</ins>}
      {change.change === "same" && <span>{after}</span>}
    </li>
  );
}

function Preview({ pageId, versionId }: { pageId: string; versionId: string }) {
  const workspace = useWorkspace();
  const version = useQuery(versionQuery(workspace.id, pageId, versionId));
  if (version.isPending) {
    return <Loading />;
  }
  if (version.isError) {
    return (
      <ErrorNotice
        error={version.error}
        retrying={version.isFetching}
        onRetry={() => void version.refetch()}
      />
    );
  }
  if (version.data.blocks.length === 0) {
    return <p className="text-sm text-muted-foreground">{copy.emptyVersion}</p>;
  }
  return (
    <section aria-label={copy.previewLabel} className="rounded-lg border border-border py-4">
      <Suspense fallback={<Loading />}>
        <VersionView blocks={version.data.blocks} />
      </Suspense>
    </section>
  );
}
