import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { trashQuery, useDeletePage, useRestorePage, type Page } from "@/api/pages";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { PageGlyph } from "@/components/pages/page-tree";
import { ItemActions, ItemCard } from "@/components/settings/page-section";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatDateTime } from "@/lib/format";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";

const copy = strings.pages.trash;

/**
 * The workspace's trash, as the user sees it: the pages they could edit,
 * latest first, each standing for the pages that went with it, to restore or
 * to delete for good.
 */
export function TrashPage() {
  const workspace = useWorkspace();
  const trash = useInfiniteQuery(trashQuery(workspace.id));
  const [done, setDone] = useState("");
  const pages = trash.data?.pages.flatMap((part) => part.pages) ?? [];

  return (
    <div className="grid gap-6">
      <PageHeader
        eyebrow={`${copy.eyebrow} · ${workspace.name}`}
        title={copy.title}
        lede={copy.lede}
      />
      <output className="block text-sm text-muted-foreground empty:hidden">{done}</output>
      {trash.isPending && <Loading />}
      {trash.isError && (
        <ErrorNotice
          error={trash.error}
          retrying={trash.isFetching}
          onRetry={() => void trash.refetch()}
        />
      )}
      {trash.isSuccess && pages.length === 0 && (
        <p className="text-sm text-muted-foreground">{copy.none}</p>
      )}
      {pages.length > 0 && (
        <ul aria-label={copy.title} className="grid gap-2">
          {pages.map((page) => (
            <TrashedPage key={page.id} page={page} onDone={setDone} />
          ))}
        </ul>
      )}
      {trash.hasNextPage && (
        <Button
          variant="outline"
          className="justify-self-start"
          disabled={trash.isFetchingNextPage}
          onClick={() => void trash.fetchNextPage()}
        >
          {copy.loadMore}
        </Button>
      )}
    </div>
  );
}

function TrashedPage({ page, onDone }: { page: Page; onDone: (message: string) => void }) {
  const workspace = useWorkspace();
  const restore = useRestorePage(workspace.id);
  const remove = useDeletePage(workspace.id);
  const [confirming, setConfirming] = useState(false);
  const title = titleOf(page);
  const busy = restore.isPending || remove.isPending;
  return (
    <ItemCard>
      <div className="grid min-w-0 gap-1">
        <Link
          to="/w/$slug/p/$pageId"
          params={{ slug: workspace.slug, pageId: page.id }}
          className="flex min-w-0 items-center gap-2 font-medium hover:underline"
        >
          <PageGlyph icon={page.icon} kind={page.kind} />
          <span className="truncate">{title}</span>
        </Link>
        {page.trashed_at && (
          <p className="text-xs text-muted-foreground">
            {copy.trashed(formatDateTime(page.trashed_at))}
          </p>
        )}
      </div>
      <ItemActions>
        <Button
          size="sm"
          aria-label={copy.restoreLabel(title)}
          disabled={busy}
          onClick={() => restore.mutate(page.id, { onSuccess: () => onDone(copy.restored(title)) })}
        >
          {copy.restore}
        </Button>
        <Button
          variant="outline"
          size="sm"
          aria-label={copy.deleteLabel(title)}
          disabled={busy}
          onClick={() => setConfirming(true)}
        >
          {copy.delete}
        </Button>
      </ItemActions>
      {(restore.isError || remove.isError) && (
        <div className="sm:col-span-2">
          <ErrorNotice error={restore.error ?? remove.error} />
        </div>
      )}
      <ConfirmDialog
        open={confirming}
        title={copy.deleteTitle(title)}
        description={copy.deleteLede}
        confirm={copy.delete}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          remove.mutate(page.id, { onSuccess: () => onDone(copy.deleted(title)) });
        }}
      />
    </ItemCard>
  );
}
