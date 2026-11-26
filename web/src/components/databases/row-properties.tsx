import { useQuery } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Pencil, Plus } from "lucide-react";
import { useCallback, useState, type ReactNode } from "react";
import { useDatabaseFeed } from "@/api/database-feed";
import {
  databaseQuery,
  rowQuery,
  useMoveProperty,
  type Database,
  type Property,
  type Row,
} from "@/api/databases";
import { hasCode } from "@/api/errors";
import type { Page } from "@/api/pages";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";
import { Cell } from "./cell";
import { DatabaseScope, useDatabaseScope } from "./database-scope";
import { FeedClosed } from "./feed-indicator";
import { Problem } from "./problem";
import { PropertyDialog } from "./property-dialog";
import { PropertyIcon } from "./property-icon";

const copy = strings.databases;

/**
 * A row's properties above its content, on its page and beside its database:
 * its unique id, then each property of the database but the title, which is
 * the page's title, each editable as in the views. They follow the database's
 * live changes. Someone who sees the row but not its database, through a
 * share on the row alone, reads it without its database: they see what every
 * row has, its id and when it was made and last changed, and no way into the
 * database.
 */
export function RowProperties({ page }: { page: Page }) {
  const workspace = useWorkspace();
  // A row's page is under its database, unless the user sees the row alone.
  const databaseId = page.parent_id ?? undefined;
  const database = useQuery({
    ...databaseQuery(workspace.id, databaseId ?? ""),
    enabled: databaseId !== undefined,
  });
  const row = useQuery(rowQuery(workspace.id, page.id));
  const feed = useDatabaseFeed(workspace.id, database.isSuccess ? databaseId : undefined);
  const [problem, setProblem] = useState<unknown>(null);
  const report = useCallback((error: unknown) => setProblem(error), []);

  if (row.isPending || (databaseId !== undefined && database.isPending)) {
    return <Loading label={copy.loading} />;
  }
  if (row.isError) {
    return (
      <ErrorNotice error={row.error} retrying={row.isFetching} onRetry={() => void row.refetch()} />
    );
  }
  if (database.isError && !hasCode(database.error, "not_found")) {
    return (
      <ErrorNotice
        error={database.error}
        retrying={database.isFetching}
        onRetry={() => void database.refetch()}
      />
    );
  }
  const known = (row.data.database_id ?? null) === null ? undefined : database.data;
  const section = (
    <section
      aria-label={copy.row.properties}
      className="grid gap-2"
      style={{ paddingInline: "var(--page-inset)" }}
    >
      <dl className="grid gap-x-4 sm:grid-cols-[minmax(0,12rem)_minmax(0,1fr)]">
        {/* A unique id property shows the id among the others; without one, the row's
            number is told apart from the properties filters name. */}
        {!known?.properties.some((property) => property.type === "unique_id") && (
          <Fact term={copy.row.uniqueId}>
            <span className="font-mono" title={copy.row.uniqueIdHint}>
              {row.data.unique_id}
            </span>
          </Fact>
        )}
        {known === undefined ? (
          <>
            <Fact term={copy.row.created}>{formatDateTime(row.data.created_at)}</Fact>
            <Fact term={copy.row.lastEdited}>{formatDateTime(row.data.last_edited_at)}</Fact>
          </>
        ) : (
          <Values database={known} row={row.data} />
        )}
      </dl>
      {known === undefined ? (
        <p className="text-sm text-muted-foreground">{copy.row.noDatabase}</p>
      ) : (
        <AddProperty />
      )}
      <FeedClosed state={feed.state} onRestart={feed.restart} />
      {problem !== null && <Problem error={problem} onDismiss={() => setProblem(null)} />}
    </section>
  );
  return known === undefined ? (
    section
  ) : (
    <DatabaseScope database={known} report={report}>
      {section}
    </DatabaseScope>
  );
}

/** One of what every row has, shown without its database: its name and its value. */
function Fact({ term, children }: { term: string; children: ReactNode }) {
  return (
    <div className="contents">
      <dt className="flex min-h-(--tap-sm) items-center text-sm text-muted-foreground">{term}</dt>
      <dd className="flex min-h-(--tap-sm) items-center text-sm">{children}</dd>
    </div>
  );
}

function Values({ database, row }: { database: Database; row: Row }) {
  const properties = database.properties.filter((property) => property.type !== "title");
  return (
    <>
      {properties.map((property, index) => (
        <div key={property.id} className="contents">
          <dt className="flex min-h-(--tap-sm) min-w-0 items-center gap-1 text-sm text-muted-foreground">
            <PropertyLabel
              property={property}
              before={properties[index - 1]}
              after={properties[index + 1]}
            />
          </dt>
          <dd className="min-w-0 pb-1 sm:pb-0">
            <Cell property={property} row={row} variant="panel" />
          </dd>
        </div>
      ))}
    </>
  );
}

type PropertyLabelProps = {
  property: Property;
  before: Property | undefined;
  after: Property | undefined;
};

/**
 * A property's name, and for those who edit the database, its menu: edit it,
 * or move it among the properties, which is the order every row's page lists
 * them in.
 */
function PropertyLabel({ property, before, after }: PropertyLabelProps) {
  const workspace = useWorkspace();
  const { database, editsDatabase, report } = useDatabaseScope();
  const move = useMoveProperty(workspace.id, database.page.id);
  const [editing, setEditing] = useState(false);
  const name = (
    <>
      <PropertyIcon kind={property.type} />
      <span className="truncate">{property.name}</span>
    </>
  );
  if (!editsDatabase) {
    return name;
  }
  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={copy.properties.menu(property.name)}
            className="flex min-h-(--tap-sm) min-w-0 items-center gap-1.5 rounded-sm px-1 text-left hover:bg-accent hover:text-foreground"
          >
            {name}
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onSelect={() => setEditing(true)}>
            <Pencil aria-hidden />
            {copy.properties.edit(property.name)}
          </DropdownMenuItem>
          {before !== undefined && (
            <DropdownMenuItem
              onSelect={() =>
                move.mutate(
                  { propertyId: property.id, placement: { before: before.id } },
                  { onError: report },
                )
              }
            >
              <ArrowUp aria-hidden />
              {copy.properties.moveUp(property.name)}
            </DropdownMenuItem>
          )}
          {after !== undefined && (
            <DropdownMenuItem
              onSelect={() =>
                move.mutate(
                  { propertyId: property.id, placement: { after: after.id } },
                  { onError: report },
                )
              }
            >
              <ArrowDown aria-hidden />
              {copy.properties.moveDown(property.name)}
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      {editing && <PropertyDialog open onOpenChange={setEditing} property={property} />}
    </>
  );
}

function AddProperty() {
  const { editsDatabase } = useDatabaseScope();
  const [adding, setAdding] = useState(false);
  if (!editsDatabase) {
    return null;
  }
  return (
    <div>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setAdding(true)}
        className="text-muted-foreground"
      >
        <Plus aria-hidden />
        {copy.properties.add}
      </Button>
      {adding && <PropertyDialog open onOpenChange={setAdding} />}
    </div>
  );
}
