import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { ArrowUpDown, Layers, ListFilter, Plus, Search, Settings2 } from "lucide-react";
import { useCallback, useId, useState, type FormEvent, type ReactNode } from "react";
import { useDatabaseFeed } from "@/api/database-feed";
import {
  findRow,
  useChangeView,
  useCreateRow,
  viewRowsQuery,
  type ChangeView,
  type Database,
  type GroupKey,
  type Property,
  type View,
} from "@/api/databases";
import { hasCode } from "@/api/errors";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { filterSize, propertyOf } from "@/lib/database-rules";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { BoardView } from "./board-view";
import { DatabaseScope, useDatabaseScope, useOpenRow } from "./database-scope";
import { FeedClosed, FeedIndicator } from "./feed-indicator";
import { FilterBuilder } from "./filter-builder";
import { GroupSettings } from "./group-settings";
import { ListView } from "./list-view";
import { Problem } from "./problem";
import { PropertiesPicker } from "./properties-picker";
import { SortBuilder } from "./sort-builder";
import { TableView } from "./table-view";
import type { ViewBodyProps } from "./view-body";
import { ViewTabs } from "./view-tabs";

const copy = strings.databases;

type Panel = "filter" | "sort" | "group" | "properties" | "find";

type DatabaseViewProps = {
  database: Database;
  /** The view to show; the first when it names none the database has. */
  viewId: string | undefined;
  onViewChange: (viewId: string) => void;
  /** Inside another page: smaller, the feed's state a dot. */
  compact?: boolean;
  /** What goes above the views, inside the database's region: an inline one's title. */
  header?: ReactNode;
};

/**
 * A database as its views show it: the tabs of its views, the settings of
 * the one in view, its filter, sorts, grouping and properties, and its rows
 * as a table, a board or a list. Every change anyone makes shows as it is
 * saved, through the database's live feed, and everything is read afresh as
 * the feed reconnects.
 */
export function DatabaseView({
  database,
  viewId,
  onViewChange,
  compact = false,
  header,
}: DatabaseViewProps) {
  const workspace = useWorkspace();
  const feed = useDatabaseFeed(workspace.id, database.page.id);
  const [problem, setProblem] = useState<unknown>(null);
  const [panel, setPanel] = useState<Panel | null>(null);
  const [filterStart, setFilterStart] = useState<{ property: Property; at: number } | null>(null);
  const panelId = useId();
  const report = useCallback((error: unknown) => setProblem(error), []);
  const startFilter = useCallback((property: Property) => {
    setFilterStart({ property, at: Date.now() });
    setPanel("filter");
  }, []);
  const view = database.views.find((entry) => entry.id === viewId) ?? database.views[0];
  if (view === undefined) {
    return null;
  }
  return (
    <DatabaseScope database={database} report={report} compact={compact} startFilter={startFilter}>
      <section aria-label={copy.label(titleOf(database.page))} className="grid min-w-0 gap-2">
        {header}
        <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 border-b border-border pb-1">
          <ViewTabs view={view} panelId={panelId} onSelect={onViewChange} />
          <Toolbar
            view={view}
            panel={panel}
            onPanel={(next) => {
              setPanel(next);
              if (next !== "filter") {
                setFilterStart(null);
              }
            }}
            filterStart={filterStart}
            feed={<FeedIndicator state={feed.state} compact={compact} />}
          />
        </div>
        <FeedClosed state={feed.state} onRestart={feed.restart} />
        {problem !== null && <Problem error={problem} onDismiss={() => setProblem(null)} />}
        <div
          role="tabpanel"
          id={panelId}
          aria-labelledby={`${panelId}-${view.id}`}
          className="min-w-0"
        >
          <ViewRows key={view.id} view={view} />
        </div>
      </section>
    </DatabaseScope>
  );
}

/** The rows of the view in view, laid out as it says, with their count and the way to add one. */
function ViewRows({ view }: { view: View }) {
  const workspace = useWorkspace();
  const { database, editsDatabase, report } = useDatabaseScope();
  const rows = useQuery(viewRowsQuery(workspace.id, database.page.id, view.id));
  const change = useChangeView(workspace.id, database.page.id);
  const create = useCreateRow(workspace.id, database);
  const [editingTitle, setEditingTitle] = useState<string | null>(null);
  const onChangeView = (body: ChangeView) =>
    change.mutate({ viewId: view.id, ...body }, { onError: report });
  const onNewRow = (key?: GroupKey) =>
    create.mutate(key === undefined ? {} : { group: { view_id: view.id, key } }, {
      onSuccess: (row) => setEditingTitle(row.id),
      onError: report,
    });

  if (rows.isPending) {
    return <Loading label={copy.loading} />;
  }
  if (rows.isError) {
    return (
      <ErrorNotice
        error={rows.error}
        title={copy.rows.failed}
        retrying={rows.isFetching}
        onRetry={() => void rows.refetch()}
      />
    );
  }
  const body: ViewBodyProps = {
    view,
    answer: rows.data,
    editingTitle,
    onEditingTitle: setEditingTitle,
    onChangeView,
    onNewRow,
  };
  const count = new Set(rows.data.rows.map((row) => row.id)).size;
  return (
    <div className="grid gap-2">
      {view.layout === "table" && <TableView {...body} />}
      {view.layout === "board" && <BoardView {...body} />}
      {view.layout === "list" && <ListView {...body} />}
      {count === 0 && view.layout !== "board" && (
        <p className="px-1 text-sm text-muted-foreground">
          {(view.filter ?? null) === null ? copy.rows.none : copy.rows.noneShown}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-3 px-1">
        {editsDatabase && view.layout !== "board" && (
          <Button
            variant="ghost"
            size="sm"
            aria-label={copy.toolbar.newRowLabel}
            disabled={create.isPending}
            onClick={() => onNewRow()}
            className="text-muted-foreground"
          >
            <Plus aria-hidden />
            {copy.toolbar.newRow}
          </Button>
        )}
        <span className="text-xs text-muted-foreground">{copy.rows.count(count)}</span>
      </div>
    </div>
  );
}

type ToolbarProps = {
  view: View;
  panel: Panel | null;
  onPanel: (panel: Panel | null) => void;
  filterStart: { property: Property; at: number } | null;
  feed: ReactNode;
};

/** The view's settings, each in its own panel, for those who edit the database; and finding a row by its id. */
function Toolbar({ view, panel, onPanel, filterStart, feed }: ToolbarProps) {
  const workspace = useWorkspace();
  const { database, editsDatabase, report } = useDatabaseScope();
  const change = useChangeView(workspace.id, database.page.id);
  const rows = useQuery({
    ...viewRowsQuery(workspace.id, database.page.id, view.id),
    enabled: false,
  });
  const save = (body: ChangeView) =>
    change.mutate({ viewId: view.id, ...body }, { onError: report });
  const conditions = filterSize(view.filter);
  const grouping = view.group ? propertyOf(database, view.group.property) : undefined;
  const opener = (name: Panel) => ({
    open: panel === name,
    onOpenChange: (open: boolean) => onPanel(open ? name : null),
  });

  return (
    <div className="flex flex-wrap items-center gap-0.5">
      {feed}
      {editsDatabase && (
        <>
          <ToolbarPanel
            {...opener("filter")}
            icon={<ListFilter aria-hidden />}
            label={conditions > 0 ? copy.toolbar.filtered(conditions) : copy.toolbar.filter}
            text={copy.toolbar.filter}
            active={conditions > 0}
          >
            <FilterBuilder
              key={filterStart?.at ?? "filter"}
              view={view}
              start={filterStart?.property}
              onSave={(filter) => save({ filter })}
            />
          </ToolbarPanel>
          <ToolbarPanel
            {...opener("sort")}
            icon={<ArrowUpDown aria-hidden />}
            label={
              view.sorts.length > 0 ? copy.toolbar.sorted(view.sorts.length) : copy.toolbar.sort
            }
            text={copy.toolbar.sort}
            active={view.sorts.length > 0}
          >
            <SortBuilder view={view} onSave={(sorts) => save({ sorts })} />
          </ToolbarPanel>
          <ToolbarPanel
            {...opener("group")}
            icon={<Layers aria-hidden />}
            label={grouping ? copy.toolbar.grouped(grouping.name) : copy.toolbar.group}
            text={copy.toolbar.group}
            active={grouping !== undefined && view.layout !== "board"}
          >
            <GroupSettings
              view={view}
              groups={rows.data?.groups ?? null}
              onSave={(group) => save({ group })}
            />
          </ToolbarPanel>
          <ToolbarPanel
            {...opener("properties")}
            icon={<Settings2 aria-hidden />}
            label={copy.toolbar.properties}
            text={copy.toolbar.properties}
            active={false}
          >
            <PropertiesPicker view={view} onSave={(properties) => save({ properties })} />
          </ToolbarPanel>
        </>
      )}
      <ToolbarPanel
        {...opener("find")}
        icon={<Search aria-hidden />}
        label={copy.find.open}
        text={null}
        active={false}
      >
        <FindRow onFound={() => onPanel(null)} />
      </ToolbarPanel>
    </div>
  );
}

type ToolbarPanelProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  icon: ReactNode;
  /** What the button is called, with what the view holds of it. */
  label: string;
  /** The word on the button, which narrow screens leave out; none for an icon alone. */
  text: string | null;
  /** Whether the view has some of it, which the button shows in the accent. */
  active: boolean;
  children: ReactNode;
};

function ToolbarPanel({
  open,
  onOpenChange,
  icon,
  label,
  text,
  active,
  children,
}: ToolbarPanelProps) {
  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size={text === null ? "icon-sm" : "sm"}
          aria-label={label}
          className={cn(active && "text-primary")}
        >
          {icon}
          {text !== null && <span className="hidden sm:inline">{text}</span>}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" aria-label={label} className="w-[min(36rem,calc(100vw-1rem))]">
        {children}
      </PopoverContent>
    </Popover>
  );
}

/** Opens the row a unique id names, `NW-42`, as people and agents name them. */
function FindRow({ onFound }: { onFound: () => void }) {
  const { database } = useDatabaseScope();
  const openRow = useOpenRow();
  const [id, setId] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [pending, setPending] = useState(false);
  const fieldId = useId();
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const wanted = id.trim();
    if (wanted === "") {
      return;
    }
    setPending(true);
    setError(null);
    try {
      const row = await findRow(database.page.id, wanted);
      onFound();
      openRow(row.id);
    } catch (failure) {
      setError(failure);
    } finally {
      setPending(false);
    }
  };
  return (
    <form onSubmit={(event) => void submit(event)} className="grid gap-2">
      <label htmlFor={fieldId} className="text-sm font-medium">
        {copy.find.label}
      </label>
      <div className="flex gap-2">
        <Input
          id={fieldId}
          placeholder={copy.find.placeholder}
          value={id}
          onChange={(event) => setId(event.target.value)}
        />
        <Button type="submit" disabled={pending}>
          {copy.find.submit}
        </Button>
      </div>
      {error !== null &&
        (hasCode(error, "not_found") ? (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">{copy.find.none(id.trim())}</p>
          </Alert>
        ) : (
          <ErrorNotice error={error} />
        ))}
    </form>
  );
}
