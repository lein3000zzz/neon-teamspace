import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Database, Maximize2, Plus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { databaseQuery, type Database as DatabaseData } from "@/api/databases";
import { hasCode } from "@/api/errors";
import { showPage, useUpdatePage } from "@/api/pages";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { edits } from "@/lib/page-rules";
import { strings } from "@/lib/strings";
import { DatabaseView } from "./database-view";
import { Problem } from "./problem";

const copy = strings.databases.block;

type DatabaseEmbedProps = {
  /** Empty for a block given no database yet. */
  databaseId: string;
  viewId: string | undefined;
  /** Whether the database for an empty block is being made. */
  creating: boolean;
  /** Makes a database for an empty block, for those who edit the page. */
  onCreate: (() => void) | undefined;
  /** Keeps the view picked in the block, for those who edit the page; others pick for themselves. */
  onViewChange: ((viewId: string) => void) | undefined;
};

/**
 * A database inside a page: its title, the way to open it as a full page, and
 * its views, compact. The ids come from the page's document, which anyone who
 * edits it may set, so a database the user may not see, or that does not
 * exist, says so as the server answers.
 */
export function DatabaseEmbed({
  databaseId,
  viewId,
  creating,
  onCreate,
  onViewChange,
}: DatabaseEmbedProps) {
  return (
    <div data-database-embed="" className="grid w-full min-w-0 gap-2 py-1">
      {databaseId === "" ? (
        <Unset creating={creating} onCreate={onCreate} />
      ) : (
        <Embedded databaseId={databaseId} viewId={viewId} onViewChange={onViewChange} />
      )}
    </div>
  );
}

function Unset({ creating, onCreate }: Pick<DatabaseEmbedProps, "creating" | "onCreate">) {
  return (
    <div className="flex flex-wrap items-center gap-2 rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground">
      <Database aria-hidden className="size-4 shrink-0" />
      <span className="min-w-0 flex-1">{creating ? copy.creating : copy.none}</span>
      {onCreate !== undefined && !creating && (
        <Button variant="outline" size="sm" onClick={onCreate}>
          <Plus aria-hidden />
          {copy.create}
        </Button>
      )}
    </div>
  );
}

type EmbeddedProps = Pick<DatabaseEmbedProps, "databaseId" | "viewId" | "onViewChange">;

function Embedded({ databaseId, viewId, onViewChange }: EmbeddedProps) {
  const workspace = useWorkspace();
  const navigate = useNavigate();
  const database = useQuery(databaseQuery(workspace.id, databaseId));
  // The view picked by someone who cannot keep it in the block, for as long as the page is open.
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const [notRenamed, setNotRenamed] = useState<unknown>(null);
  if (database.isPending) {
    return <Loading label={strings.databases.loading} />;
  }
  if (database.isError) {
    if (hasCode(database.error, "not_found", "invalid_input")) {
      return (
        <div className="grid gap-0.5 rounded-md border border-dashed border-border px-3 py-2 text-sm">
          <p className="flex items-center gap-2 text-foreground">
            <Database aria-hidden className="size-4 shrink-0 text-muted-foreground" />
            {copy.missing}
          </p>
          <p className="text-muted-foreground">{copy.missingLede}</p>
        </div>
      );
    }
    return (
      <ErrorNotice
        error={database.error}
        retrying={database.isFetching}
        onRetry={() => void database.refetch()}
      />
    );
  }
  const page = database.data.page;
  return (
    <DatabaseView
      database={database.data}
      viewId={onViewChange === undefined ? (picked ?? viewId) : viewId}
      onViewChange={onViewChange ?? setPicked}
      compact
      header={
        <>
          <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
            <EmbedTitle database={database.data} onFailed={(error) => setNotRenamed(error)} />
            <Button
              variant="ghost"
              size="sm"
              className="text-muted-foreground"
              onClick={() =>
                void navigate({
                  to: "/w/$slug/p/$pageId",
                  params: { slug: workspace.slug, pageId: page.id },
                  search: {},
                })
              }
            >
              <Maximize2 aria-hidden />
              {copy.openFull}
            </Button>
          </div>
          {notRenamed !== null && (
            <Problem error={notRenamed} onDismiss={() => setNotRenamed(null)} />
          )}
        </>
      }
    />
  );
}

/**
 * The database's title over its views, typed in place by those who edit it:
 * the sidebar shows it as it is typed, as for a page's title; Enter or leaving
 * it saves, and a refusal says why below it. A title changed elsewhere shows
 * here unless it is being typed.
 */
function EmbedTitle({
  database,
  onFailed,
}: {
  database: DatabaseData;
  onFailed: (error: unknown) => void;
}) {
  const workspace = useWorkspace();
  const queryClient = useQueryClient();
  const page = database.page;
  const update = useUpdatePage(workspace.id, page.id, onFailed);
  const [title, setTitle] = useState(page.title);
  const sent = useRef(page.title);
  const field = useRef<HTMLInputElement>(null);
  const icon = page.icon ? <span aria-hidden>{page.icon}</span> : null;

  useEffect(() => {
    if (document.activeElement !== field.current) {
      setTitle(page.title);
      sent.current = page.title;
    }
  }, [page.title]);

  if (!edits(page.access) || (page.trashed_at ?? null) !== null) {
    return (
      <h3 className="flex min-w-0 items-center gap-1.5 text-lg font-semibold">
        {icon}
        <span className="truncate">
          {page.title.trim() === "" ? strings.pages.untitled : page.title}
        </span>
      </h3>
    );
  }
  const save = () => {
    if (title !== sent.current) {
      sent.current = title;
      update.mutate({ title });
    }
  };
  return (
    <h3 className="flex min-w-0 flex-1 items-center gap-1.5 text-lg font-semibold">
      {icon}
      <input
        ref={field}
        aria-label={copy.title}
        placeholder={strings.pages.untitled}
        maxLength={500}
        value={title}
        onChange={(event) => {
          const text = event.target.value.replaceAll(/[\r\n]+/g, " ");
          setTitle(text);
          showPage(queryClient, workspace.id, page.id, { title: text });
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.nativeEvent.isComposing) {
            event.preventDefault();
            save();
          }
        }}
        onBlur={save}
        className="min-w-0 flex-1 rounded-sm bg-transparent px-1 outline-offset-2 placeholder:text-muted-foreground"
      />
    </h3>
  );
}
