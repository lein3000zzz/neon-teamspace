import { queryOptions, useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";
import type { Page, Placement } from "./pages";

export type Database = Schemas["Database"];
export type Property = Schemas["Property"];
export type PropertyKind = Schemas["PropertyKind"];
export type PropertyOption = Schemas["PropertyOption"];
export type OptionChange = Schemas["OptionChange"];
export type StatusGroup = Schemas["StatusGroup"];
export type NumberFormat = Schemas["NumberFormat"];
export type View = Schemas["View"];
export type ViewProperty = Schemas["ViewProperty"];
export type Layout = Schemas["Layout"];
export type Filter = Schemas["Filter"];
export type Condition = Schemas["Condition"];
export type Operator = Schemas["Operator"];
export type Sort = Schemas["Sort"];
export type Grouping = Schemas["Grouping"];
export type QueryGroup = Schemas["QueryGroup"];
export type AddProperty = Schemas["AddProperty"];
export type ChangeProperty = Schemas["ChangeProperty"];
export type CreateView = Schemas["CreateView"];
export type ChangeView = Schemas["ChangeView"];
export type CreateDatabase = Schemas["CreateDatabase"];

/**
 * A row's values by property id. The API describes them as an open object, so
 * the generated type has no room for them; their shapes depend on the
 * property's type, as the databases reference lists.
 */
export type Values = Record<string, unknown>;

/** A row of a database, with its values. */
export type Row = Omit<Schemas["Row"], "values"> & { values: Values };

/** Every row a view lists, read part after part, with its groups when it groups. */
export type ViewRows = { rows: Row[]; groups: QueryGroup[] | null; total: number };

/** A group of a view, as rows are added to or moved into one. */
export type GroupKey = string | null;

/** Rows in one part of a query: the most the server answers with at once. */
const ROWS_PART = 1000;

/**
 * The reader's offset from UTC in minutes, which the server tells days at:
 * what today is for a relative date, and the day a moment falls on.
 */
export function utcOffsetMinutes(): number {
  return -new Date().getTimezoneOffset();
}

const databasePath = (id: string) => ({ params: { path: { id } } });

/** The body the API takes for values, which its generated type cannot describe. */
const valuesBody = (values: Values) => values as Record<string, never>;

/** A row as the API sends it, its values typed as they are. */
const asRow = (row: Schemas["Row"]): Row => row as Row;

/** A database's page, its properties and its views. Not found unless the user sees it. */
export function databaseQuery(workspaceId: string, databaseId: string) {
  return queryOptions({
    queryKey: keys.database(workspaceId, databaseId),
    queryFn: ({ signal }) =>
      call(api.GET("/api/v1/databases/{id}", { ...databasePath(databaseId), signal })),
  });
}

/**
 * The rows a view lists, filtered, sorted and grouped by the server as the
 * view says, every part of them: a database holds at most 10,000 rows, so a
 * view comes in at most ten parts.
 */
export function viewRowsQuery(workspaceId: string, databaseId: string, viewId: string) {
  return queryOptions({
    queryKey: keys.viewRows(workspaceId, databaseId, viewId),
    queryFn: async ({ signal }): Promise<ViewRows> => {
      const part = (after: string | undefined) =>
        call(
          api.POST("/api/v1/databases/{id}/query", {
            ...databasePath(databaseId),
            body: {
              view_id: viewId,
              limit: ROWS_PART,
              utc_offset_minutes: utcOffsetMinutes(),
              ...(after === undefined ? {} : { after }),
            },
            signal,
          }),
        );
      const first = await part(undefined);
      return {
        rows: await allRows(first, part),
        groups: first.groups ?? null,
        total: first.total,
      };
    },
  });
}

type QueryPart = Schemas["QueryAnswer"];

/** The rows of a query's part and of every part after it, read one after the other. */
async function allRows(
  answer: QueryPart,
  part: (after: string | undefined) => Promise<QueryPart>,
): Promise<Row[]> {
  const rows = answer.rows.map(asRow);
  const next = answer.next ?? undefined;
  return next === undefined ? rows : [...rows, ...(await allRows(await part(next), part))];
}

/** One row with its values, as its page shows them. */
export function rowQuery(workspaceId: string, rowId: string) {
  return queryOptions({
    queryKey: keys.row(workspaceId, rowId),
    queryFn: async ({ signal }) =>
      asRow(await call(api.GET("/api/v1/rows/{id}", { ...databasePath(rowId), signal }))),
  });
}

/** The row a unique id names, `NW-42`, `nw-42` or `42`, in a database. */
export async function findRow(databaseId: string, uniqueId: string): Promise<Row> {
  return asRow(
    await call(
      api.GET("/api/v1/databases/{id}/rows/{unique_id}", {
        params: { path: { id: databaseId, unique_id: uniqueId } },
      }),
    ),
  );
}

/** How long changes settle before the views' rows are read again, so a burst reads once. */
export const REFRESH_MS = 250;

const refreshTimers = new WeakMap<QueryClient, Map<string, ReturnType<typeof setTimeout>>>();

/**
 * Reads every view's rows of a database again, shortly: the server filters,
 * sorts and groups them, so a change shown here at once settles in its place
 * once it answers. Several changes in a row read once.
 */
export function refreshRows(queryClient: QueryClient, workspaceId: string, databaseId: string) {
  let timers = refreshTimers.get(queryClient);
  if (timers === undefined) {
    timers = new Map();
    refreshTimers.set(queryClient, timers);
  }
  const key = `${workspaceId}:${databaseId}`;
  clearTimeout(timers.get(key));
  timers.set(
    key,
    setTimeout(() => {
      timers.delete(key);
      void queryClient.invalidateQueries({ queryKey: keys.databaseRows(workspaceId, databaseId) });
    }, REFRESH_MS),
  );
}

/** Writes a change to a row into every view that lists it, and into the row itself. */
export function patchRow(
  queryClient: QueryClient,
  workspaceId: string,
  databaseId: string,
  rowId: string,
  patch: (row: Row) => Row,
) {
  queryClient.setQueriesData<ViewRows>(
    { queryKey: keys.databaseRows(workspaceId, databaseId) },
    (answer) =>
      answer && {
        ...answer,
        rows: answer.rows.map((row) => (row.id === rowId ? patch(row) : row)),
      },
  );
  queryClient.setQueryData<Row>(keys.row(workspaceId, rowId), (row) => row && patch(row));
}

/** Takes a row out of every view, as it goes to the trash. */
export function dropRow(
  queryClient: QueryClient,
  workspaceId: string,
  databaseId: string,
  rowId: string,
) {
  queryClient.setQueriesData<ViewRows>(
    { queryKey: keys.databaseRows(workspaceId, databaseId) },
    (answer) => {
      if (answer === undefined) {
        return answer;
      }
      const leaving = answer.rows.filter((row) => row.id === rowId);
      if (leaving.length === 0) {
        return answer;
      }
      return {
        rows: answer.rows.filter((row) => row.id !== rowId),
        total: Math.max(0, answer.total - leaving.length),
        groups:
          answer.groups?.map((group) => ({
            ...group,
            count: Math.max(
              0,
              group.count - leaving.filter((row) => (row.group ?? null) === group.key).length,
            ),
          })) ?? null,
      };
    },
  );
}

/**
 * The groups a row is in, in a view grouped by `property`: the option it has,
 * or each person it names, or the group of rows without a value.
 */
export function groupKeys(property: Property, row: Pick<Row, "values">): GroupKey[] {
  const value = row.values[property.id];
  if (property.type === "person") {
    const people = Array.isArray(value)
      ? value.filter((entry): entry is string => typeof entry === "string")
      : [];
    return people.length === 0 ? [null] : people;
  }
  return [typeof value === "string" ? value : null];
}

/**
 * Adds a new row to the views that show every row: last, in each of its
 * groups that shows, for a view that groups. A view that filters places it
 * once its rows are read again, as the filter is the server's to apply.
 */
export function addRow(
  queryClient: QueryClient,
  workspaceId: string,
  database: Database,
  row: Row,
) {
  for (const view of database.views) {
    if ((view.filter ?? null) !== null) {
      continue;
    }
    const grouping = view.group
      ? database.properties.find((property) => property.id === view.group?.property)
      : undefined;
    queryClient.setQueryData<ViewRows>(
      keys.viewRows(workspaceId, database.page.id, view.id),
      (answer) => {
        if (answer === undefined || answer.rows.some((entry) => entry.id === row.id)) {
          return answer;
        }
        if (grouping === undefined || answer.groups === null) {
          return { ...answer, rows: [...answer.rows, row], total: answer.total + 1 };
        }
        const groups = groupKeys(grouping, row);
        const shown = answer.groups.filter(
          (group) => !group.hidden && groups.includes(group.key ?? null),
        );
        return {
          rows: [...answer.rows, ...shown.map((group) => ({ ...row, group: group.key ?? null }))],
          total: answer.total + shown.length,
          groups: answer.groups.map((group) =>
            groups.includes(group.key ?? null) ? { ...group, count: group.count + 1 } : group,
          ),
        };
      },
    );
  }
}

/** The values of a change written over a row's: those named, null clearing one. */
export function withValues(row: Row, values: Values, titleId: string | undefined): Row {
  const next: Values = { ...row.values };
  for (const [id, value] of Object.entries(values)) {
    if (value === null) {
      delete next[id];
    } else {
      next[id] = value;
    }
  }
  const title = titleId === undefined ? undefined : values[titleId];
  return {
    ...row,
    values: next,
    ...(title === undefined ? {} : { title: typeof title === "string" ? title : "" }),
  };
}

/** Writes a row's new title wherever its page is shown, as a rename does. */
function showRowTitle(queryClient: QueryClient, workspaceId: string, row: Row) {
  queryClient.setQueryData<Page>(keys.page(workspaceId, row.id), (page) =>
    page === undefined ? page : { ...page, title: row.title },
  );
}

/**
 * Makes a database: a full page in a place, or, with `inline` and a parent,
 * one embedded in that page. It comes with a title property, a status, a
 * table and a board.
 */
export function useCreateDatabase(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateDatabase) =>
      call(api.POST("/api/v1/workspaces/{id}/databases", { ...databasePath(workspaceId), body })),
    onSuccess: (database) => {
      queryClient.setQueryData(keys.database(workspaceId, database.page.id), database);
      queryClient.setQueryData(keys.page(workspaceId, database.page.id), database.page);
      void queryClient.invalidateQueries({ queryKey: keys.pageTrees(workspaceId) });
    },
  });
}

/** Reads the database again, without its rows, as its schema or views changed. */
function refreshSchema(queryClient: QueryClient, workspaceId: string, databaseId: string) {
  return queryClient.invalidateQueries({
    queryKey: keys.database(workspaceId, databaseId),
    exact: true,
  });
}

/** Reads the database and every row again, as a change converted or cleared values. */
function refreshAll(queryClient: QueryClient, workspaceId: string, databaseId: string) {
  void queryClient.invalidateQueries({ queryKey: keys.database(workspaceId, databaseId) });
  void queryClient.invalidateQueries({ queryKey: keys.rows(workspaceId) });
}

export function useAddProperty(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: AddProperty) =>
      call(api.POST("/api/v1/databases/{id}/properties", { ...databasePath(databaseId), body })),
    onSuccess: () => refreshSchema(queryClient, workspaceId, databaseId),
  });
}

/**
 * Renames a property, changes its type or settings, or its options. A change
 * that would lose values answers `409 lossy_change` until it is sent again
 * with `confirm`.
 */
export function useChangeProperty(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ propertyId, ...body }: ChangeProperty & { propertyId: string }) =>
      call(
        api.PATCH("/api/v1/databases/{id}/properties/{property_id}", {
          params: { path: { id: databaseId, property_id: propertyId } },
          body,
        }),
      ),
    onSuccess: () => refreshAll(queryClient, workspaceId, databaseId),
  });
}

export function useMoveProperty(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ propertyId, placement }: { propertyId: string; placement: Placement }) =>
      call(
        api.POST("/api/v1/databases/{id}/properties/{property_id}/move", {
          params: { path: { id: databaseId, property_id: propertyId } },
          body: { placement },
        }),
      ),
    onSuccess: () => refreshSchema(queryClient, workspaceId, databaseId),
  });
}

/** Deletes a property, any but the title, with its values on every row. */
export function useDeleteProperty(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (propertyId: string) =>
      send(
        api.DELETE("/api/v1/databases/{id}/properties/{property_id}", {
          params: { path: { id: databaseId, property_id: propertyId } },
        }),
      ),
    onSuccess: () => refreshAll(queryClient, workspaceId, databaseId),
  });
}

/** Writes a view the server answered with into the database, in its place. */
function storeView(queryClient: QueryClient, workspaceId: string, view: View) {
  queryClient.setQueryData<Database>(
    keys.database(workspaceId, view.database_id),
    (database) =>
      database && {
        ...database,
        views: database.views.some((entry) => entry.id === view.id)
          ? database.views.map((entry) => (entry.id === view.id ? view : entry))
          : [...database.views, view],
      },
  );
}

export function useCreateView(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateView) =>
      call(api.POST("/api/v1/databases/{id}/views", { ...databasePath(databaseId), body })),
    onSuccess: (view) => {
      storeView(queryClient, workspaceId, view);
      void refreshSchema(queryClient, workspaceId, databaseId);
    },
  });
}

/**
 * Changes a view's name, layout, filter, sorts, grouping or properties. What
 * it shows changes at once, and its rows are read again once the server
 * answers, as the filter, the sorts or the grouping may differ.
 */
export function useChangeView(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ viewId, ...body }: ChangeView & { viewId: string }) =>
      call(
        api.PATCH("/api/v1/databases/{id}/views/{view_id}", {
          params: { path: { id: databaseId, view_id: viewId } },
          body,
        }),
      ),
    onMutate: ({ viewId, ...body }) => {
      queryClient.setQueryData<Database>(
        keys.database(workspaceId, databaseId),
        (database) =>
          database && {
            ...database,
            views: database.views.map((view) =>
              view.id === viewId ? withViewChange(view, body) : view,
            ),
          },
      );
    },
    onError: () => refreshSchema(queryClient, workspaceId, databaseId),
    onSuccess: (view, body) => {
      storeView(queryClient, workspaceId, view);
      if (
        body.filter !== undefined ||
        body.sorts !== undefined ||
        body.group !== undefined ||
        body.layout !== undefined
      ) {
        void queryClient.invalidateQueries({
          queryKey: keys.viewRows(workspaceId, databaseId, view.id),
        });
      }
    },
  });
}

/** A view with a change applied as the server applies it, for showing it before it answers. */
export function withViewChange(view: View, change: ChangeView): View {
  return {
    ...view,
    ...(typeof change.name === "string" ? { name: change.name } : {}),
    ...(change.layout ? { layout: change.layout } : {}),
    ...(change.filter === undefined ? {} : { filter: change.filter }),
    ...(change.sorts ? { sorts: change.sorts } : {}),
    ...(change.group === undefined ? {} : { group: change.group }),
    ...(change.properties ? { properties: change.properties } : {}),
  };
}

export function useMoveView(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ viewId, placement }: { viewId: string; placement: Placement }) =>
      call(
        api.POST("/api/v1/databases/{id}/views/{view_id}/move", {
          params: { path: { id: databaseId, view_id: viewId } },
          body: { placement },
        }),
      ),
    onSuccess: () => refreshSchema(queryClient, workspaceId, databaseId),
  });
}

/** Copies a view right after itself, named after it with “(copy)”. */
export function useDuplicateView(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (viewId: string) =>
      call(
        api.POST("/api/v1/databases/{id}/views/{view_id}/duplicate", {
          params: { path: { id: databaseId, view_id: viewId } },
          body: {},
        }),
      ),
    onSuccess: () => refreshSchema(queryClient, workspaceId, databaseId),
  });
}

/** Deletes a view; a database keeps at least one. */
export function useDeleteView(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (viewId: string) =>
      send(
        api.DELETE("/api/v1/databases/{id}/views/{view_id}", {
          params: { path: { id: databaseId, view_id: viewId } },
        }),
      ),
    onSuccess: (_, viewId) => {
      queryClient.setQueryData<Database>(
        keys.database(workspaceId, databaseId),
        (database) =>
          database && {
            ...database,
            views: database.views.filter((view) => view.id !== viewId),
          },
      );
      queryClient.removeQueries({ queryKey: keys.viewRows(workspaceId, databaseId, viewId) });
      void refreshSchema(queryClient, workspaceId, databaseId);
    },
  });
}

export type NewRow = {
  values?: Values;
  placement?: Placement;
  /** The group of a view it goes in, as a card added to a board's column. */
  group?: { view_id: string; key: GroupKey };
};

/** Adds a row, shown at once in the views that list every row, then read again in place. */
export function useCreateRow(workspaceId: string, database: Database) {
  const queryClient = useQueryClient();
  const databaseId = database.page.id;
  return useMutation({
    mutationFn: async ({ values, ...body }: NewRow) =>
      asRow(
        await call(
          api.POST("/api/v1/databases/{id}/rows", {
            ...databasePath(databaseId),
            body: { ...body, ...(values === undefined ? {} : { values: valuesBody(values) }) },
          }),
        ),
      ),
    onSuccess: (row) => {
      addRow(queryClient, workspaceId, database, row);
      refreshRows(queryClient, workspaceId, databaseId);
    },
  });
}

/**
 * Changes a row's values: those named, null clearing one. Shown at once, the
 * last write winning on the server; a refusal reads the rows again.
 */
export function useChangeRow(workspaceId: string, database: Database) {
  const queryClient = useQueryClient();
  const databaseId = database.page.id;
  const titleId = database.properties.find((property) => property.type === "title")?.id;
  return useMutation({
    mutationFn: async ({ rowId, values }: { rowId: string; values: Values }) =>
      asRow(
        await call(
          api.PATCH("/api/v1/rows/{id}", {
            ...databasePath(rowId),
            body: { values: valuesBody(values) },
          }),
        ),
      ),
    onMutate: ({ rowId, values }) =>
      patchRow(queryClient, workspaceId, databaseId, rowId, (row) =>
        withValues(row, values, titleId),
      ),
    onError: (_, { rowId }) => {
      void queryClient.invalidateQueries({ queryKey: keys.row(workspaceId, rowId) });
      void queryClient.invalidateQueries({ queryKey: keys.databaseRows(workspaceId, databaseId) });
    },
    onSuccess: (answer) => {
      patchRow(queryClient, workspaceId, databaseId, answer.id, (row) => ({
        ...answer,
        ...(row.group === undefined ? {} : { group: row.group }),
      }));
      showRowTitle(queryClient, workspaceId, answer);
      refreshRows(queryClient, workspaceId, databaseId);
    },
  });
}

export type RowMove = {
  rowId: string;
  placement?: Placement;
  group?: { view_id: string; from: GroupKey; to: GroupKey };
};

/**
 * Moves a row among the rows, or to another group of a view. The caller
 * shows the move at once; the rows are read again shortly once the server
 * takes it, and at once when it refuses it.
 */
export function useMoveRow(workspaceId: string, databaseId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({ rowId, ...body }: RowMove) =>
      asRow(await call(api.POST("/api/v1/rows/{id}/move", { ...databasePath(rowId), body }))),
    onError: () =>
      queryClient.invalidateQueries({ queryKey: keys.databaseRows(workspaceId, databaseId) }),
    onSuccess: () => refreshRows(queryClient, workspaceId, databaseId),
  });
}
