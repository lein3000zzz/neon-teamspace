import { http, HttpResponse, type HttpResponseResolver } from "msw";
import type { Schemas } from "@/api/client";
import type { FeedEvent } from "@/api/database-feed";
import type {
  ChangeView,
  CreateView,
  Database,
  Property,
  PropertyOption,
  QueryGroup,
  Row,
  Values,
  View,
} from "@/api/databases";
import type { Page, Placement } from "@/api/pages";
import { ada, apiError } from "./fixtures";
import { page } from "./pages";
import { feedLink, server } from "./server";
import { people, TEAMSPACE_IDS, type Recorded } from "./workspaces";

const CREATED = "2026-11-20T09:00:00Z";
const EDITED = "2026-11-21T09:00:00Z";

/** Fixed ids for the database of the tests, its properties, views, options and rows. */
export const DB = {
  tasks: "0193a5b2-0000-7000-8000-0000000000d1",
  name: "0193a5b2-0000-7000-8000-0000000000e1",
  status: "0193a5b2-0000-7000-8000-0000000000e2",
  points: "0193a5b2-0000-7000-8000-0000000000e3",
  owner: "0193a5b2-0000-7000-8000-0000000000e4",
  due: "0193a5b2-0000-7000-8000-0000000000e5",
  tags: "0193a5b2-0000-7000-8000-0000000000e6",
  done: "0193a5b2-0000-7000-8000-0000000000e7",
  link: "0193a5b2-0000-7000-8000-0000000000e8",
  ticket: "0193a5b2-0000-7000-8000-0000000000e9",
  notStarted: "0193a5b2-0000-7000-8000-0000000000f1",
  inProgress: "0193a5b2-0000-7000-8000-0000000000f2",
  finished: "0193a5b2-0000-7000-8000-0000000000f3",
  bug: "0193a5b2-0000-7000-8000-0000000000f4",
  idea: "0193a5b2-0000-7000-8000-0000000000f5",
  /** An option no sample property has, which tests add as someone else would. */
  chore: "0193a5b2-0000-7000-8000-0000000000f6",
  table: "0193a5b2-0000-7000-8000-0000000000c1",
  board: "0193a5b2-0000-7000-8000-0000000000c2",
  list: "0193a5b2-0000-7000-8000-0000000000c3",
  docs: "0193a5b2-0000-7000-8000-0000000000b4",
  login: "0193a5b2-0000-7000-8000-0000000000b5",
  launch: "0193a5b2-0000-7000-8000-0000000000b6",
} as const;

/** The groups a row is in, by a grouping property. */
function keysOf(grouping: Property, entry: Row): (string | null)[] {
  const value = entry.values[grouping.id];
  if (grouping.type === "person") {
    return Array.isArray(value) && value.length > 0 ? (value as string[]) : [null];
  }
  return [typeof value === "string" ? value : null];
}

/** Places one of a list beside another, the list's positions given afresh. */
function placeAmong<T extends { id: string; position: string }>(
  list: T[],
  target: T,
  placement: Placement,
): T[] {
  const rest = list.filter((entry) => entry.id !== target.id);
  let at = rest.length;
  if (placement === "first") {
    at = 0;
  } else if (typeof placement === "object" && "before" in placement) {
    at = rest.findIndex((entry) => entry.id === placement.before);
  } else if (typeof placement === "object" && "after" in placement) {
    at = rest.findIndex((entry) => entry.id === placement.after) + 1;
  }
  const next = [...rest.slice(0, at), target, ...rest.slice(at)];
  next.forEach((entry, index) => {
    entry.position = `a${index}`;
  });
  return next;
}

/** Gives rows the positions of their order. */
function renumber(rows: Row[]) {
  rows.forEach((entry, index) => {
    entry.position = `a${index}`;
  });
}

/** What the tests make, in order: properties, views, rows and databases. */
const MADE = [
  "0193a5b2-0000-7000-8000-000000000101",
  "0193a5b2-0000-7000-8000-000000000102",
  "0193a5b2-0000-7000-8000-000000000103",
  "0193a5b2-0000-7000-8000-000000000104",
  "0193a5b2-0000-7000-8000-000000000105",
] as const;

export function properties(): Property[] {
  return [
    { id: DB.name, name: "Name", type: "title" },
    {
      id: DB.status,
      name: "Status",
      type: "status",
      options: [
        { id: DB.notStarted, name: "Not started", group: "to_do" },
        { id: DB.inProgress, name: "In progress", hue: 240, group: "in_progress" },
        { id: DB.finished, name: "Done", hue: 150, group: "done" },
      ],
    },
    { id: DB.points, name: "Points", type: "number", number_format: "plain" },
    { id: DB.owner, name: "Owner", type: "person" },
    { id: DB.due, name: "Due", type: "date" },
    {
      id: DB.tags,
      name: "Tags",
      type: "multi_select",
      options: [
        { id: DB.bug, name: "Bug", hue: 30 },
        { id: DB.idea, name: "Idea", hue: 270 },
      ],
    },
    { id: DB.done, name: "Shipped", type: "checkbox" },
    { id: DB.link, name: "Link", type: "url" },
    { id: DB.ticket, name: "ID", type: "unique_id", prefix: "NW" },
  ];
}

const shown = (ids: string[], visible: boolean) => ids.map((property) => ({ property, visible }));

export function views(): View[] {
  const all = properties().map((property) => property.id);
  return [
    {
      id: DB.table,
      database_id: DB.tasks,
      position: "a0",
      name: "Table",
      layout: "table",
      sorts: [],
      properties: shown(all, true),
    },
    {
      id: DB.board,
      database_id: DB.tasks,
      position: "a1",
      name: "Board",
      layout: "board",
      sorts: [],
      group: { property: DB.status, order: [], hidden: [] },
      properties: [
        ...shown([DB.name, DB.points, DB.owner], true),
        ...shown(
          all.slice(1).filter((id) => id !== DB.points && id !== DB.owner),
          false,
        ),
      ],
    },
    {
      id: DB.list,
      database_id: DB.tasks,
      position: "a2",
      name: "List",
      layout: "list",
      sorts: [],
      properties: [...shown([DB.name, DB.status], true), ...shown(all.slice(2), false)],
    },
  ];
}

/** The database's page, in General. */
export function tasksPage(access: Page["access"] = "edit"): Page {
  return {
    ...page({ id: DB.tasks, title: "Tasks", teamspace: TEAMSPACE_IDS.general, access }, "a3"),
    kind: "database",
  };
}

type RowSeed = {
  id: string;
  number: number;
  title: string;
  values?: Values;
  access?: Row["access"];
};

export function row(seed: RowSeed, position: string): Row {
  return {
    id: seed.id,
    database_id: DB.tasks,
    number: seed.number,
    unique_id: `NW-${seed.number}`,
    title: seed.title,
    position,
    values: { ...(seed.title === "" ? {} : { [DB.name]: seed.title }), ...seed.values },
    created_at: CREATED,
    created_by: ada.id,
    last_edited_at: EDITED,
    last_edited_by: ada.id,
    access: seed.access ?? "edit",
  };
}

/** Write docs, in progress, by Ada; Fix login, not started; Launch, with no status. */
export function sampleRows(): Row[] {
  return [
    row(
      {
        id: DB.docs,
        number: 1,
        title: "Write docs",
        values: { [DB.status]: DB.inProgress, [DB.points]: 3, [DB.owner]: [people.ada.id] },
      },
      "a0",
    ),
    row(
      {
        id: DB.login,
        number: 2,
        title: "Fix login",
        values: { [DB.status]: DB.notStarted, [DB.tags]: [DB.bug], [DB.done]: true },
      },
      "a1",
    ),
    row({ id: DB.launch, number: 3, title: "Launch" }, "a2"),
  ];
}

/** A row's page, under the database. */
export function rowPage(entry: Row): Page {
  return {
    ...page({ id: entry.id, title: entry.title, teamspace: TEAMSPACE_IDS.general }, entry.position),
    kind: "row",
    parent_id: DB.tasks,
  };
}

/** The pages of the sample, with the database and its rows' pages beside the others. */
export function databasePages(base: Page[], access: Page["access"] = "edit"): Page[] {
  return [...base, tasksPage(access), ...sampleRows().map(rowPage)];
}

type Options = {
  rows?: Row[];
  views?: View[];
  properties?: Property[];
  access?: Page["access"];
};

const missing = () => apiError(404, "not_found", "no such database");

/**
 * The database routes of the server for Tasks, holding what changes, so a view
 * reads back what it did: queries list the rows in manual order, grouped as
 * the view says, with its hidden groups left out, and ignore filters and
 * sorts, which the server applies. Every request that changes something, and
 * every query, is recorded. Call beside fakeWorkspaces() and fakePages().
 */
export function fakeDatabase(options: Options = {}) {
  // A viewer of the database views its rows too, unless a share says more.
  const seeded = (options.rows ?? sampleRows()).map((entry) =>
    options.access === "view" ? Object.assign({}, entry, { access: "view" as const }) : entry,
  );
  const state = {
    page: tasksPage(options.access),
    properties: options.properties ?? properties(),
    views: options.views ?? views(),
    rows: seeded,
    next: (options.rows ?? sampleRows()).length + 1,
  };
  const changes: Recorded[] = [];
  const queries: Schemas["QueryRows"][] = [];
  const created: Database[] = [];
  let count = 0;
  const nextId = () => {
    const id = MADE[count] ?? MADE[0];
    count += 1;
    return id;
  };

  const record = (
    resolver: (body: unknown, params: Record<string, string>) => Response | Promise<Response>,
  ) =>
    (async ({ request, params }) => {
      const text = await request.text();
      const body: unknown = text === "" ? undefined : JSON.parse(text);
      changes.push({ method: request.method, path: new URL(request.url).pathname, body });
      return resolver(body, params as Record<string, string>);
    }) satisfies HttpResponseResolver;

  const database = (): Database => ({
    page: state.page,
    inline: false,
    properties: state.properties,
    views: state.views,
  });
  const view = (id: string | undefined) => state.views.find((entry) => entry.id === id);
  const property = (id: string | undefined) =>
    state.properties.find((entry) => entry.id === id || entry.name === id);
  const ordered = () =>
    state.rows.toSorted((a, b) => (a.position < b.position ? -1 : a.position > b.position ? 1 : 0));

  /** Values named by id or name, options by id or name, as the server takes them. */
  const resolved = (values: Values): Values => {
    const out: Values = {};
    for (const [key, value] of Object.entries(values)) {
      const found = property(key);
      if (found === undefined) {
        continue;
      }
      const option = (name: unknown) =>
        found.options?.find((entry) => entry.id === name || entry.name === name)?.id ?? name;
      out[found.id] =
        found.type === "select" || found.type === "status"
          ? option(value)
          : found.type === "multi_select" && Array.isArray(value)
            ? value.map(option)
            : value;
    }
    return out;
  };

  const apply = (target: Row, values: Values) => {
    for (const [id, value] of Object.entries(resolved(values))) {
      if (value === null) {
        delete target.values[id];
      } else {
        target.values[id] = value;
      }
      if (property(id)?.type === "title") {
        target.title = typeof value === "string" ? value : "";
      }
    }
    target.last_edited_at = EDITED;
  };

  const answer = (viewId: string | undefined) => {
    const shownView = view(viewId);
    const rows = ordered();
    const grouping = shownView?.group ? property(shownView.group.property) : undefined;
    if (shownView === undefined || grouping === undefined) {
      return { rows, groups: null, total: rows.length };
    }
    const natural: (string | null)[] =
      grouping.type === "person"
        ? [
            null,
            ...new Set(
              rows.flatMap((entry) => keysOf(grouping, entry)).filter((key) => key !== null),
            ),
          ]
        : [null, ...(grouping.options ?? []).map((option) => option.id)];
    const order = shownView.group?.order ?? [];
    const keys = [
      ...order.filter((key) => natural.includes(key)),
      ...natural.filter((key) => !order.includes(key)),
    ];
    const hidden = new Set(shownView.group?.hidden ?? []);
    const groups: QueryGroup[] = keys.map((key) => ({
      key,
      hidden: hidden.has(key),
      count: rows.filter((entry) => keysOf(grouping, entry).includes(key)).length,
    }));
    const listed = keys
      .filter((key) => !hidden.has(key))
      .flatMap((key) =>
        rows
          .filter((entry) => keysOf(grouping, entry).includes(key))
          .map((entry) => Object.assign({}, entry, { group: key })),
      );
    return { rows: listed, groups, total: listed.length };
  };

  const base = "/api/v1/databases/:id";
  server.use(
    http.get(base, ({ params }) => {
      if (params["id"] === DB.tasks) {
        return HttpResponse.json(database());
      }
      const made = created.find((entry) => entry.page.id === params["id"]);
      return made === undefined ? missing() : HttpResponse.json(made);
    }),
    http.post(`${base}/query`, async ({ request, params }) => {
      const body = (await request.json()) as Schemas["QueryRows"];
      if (params["id"] !== DB.tasks) {
        const made = created.some((entry) => entry.page.id === params["id"]);
        return made ? HttpResponse.json({ rows: [], groups: null, total: 0 }) : missing();
      }
      queries.push(body);
      return HttpResponse.json(answer(body.view_id ?? undefined));
    }),
    http.post(
      `${base}/rows`,
      record((body) => {
        const input = body as Schemas["CreateRow"] & { values?: Values };
        const id = nextId();
        const made = row({ id, number: state.next, title: "" }, "z");
        state.next += 1;
        apply(made, input.values ?? {});
        const group = input.group;
        const grouping = group ? property(view(group.view_id)?.group?.property) : undefined;
        if (group && grouping) {
          if (group.key === null || group.key === undefined) {
            delete made.values[grouping.id];
          } else {
            made.values[grouping.id] = grouping.type === "person" ? [group.key] : group.key;
          }
        }
        state.rows = placeAmong(ordered(), made, input.placement ?? "last");
        return HttpResponse.json(made, { status: 201 });
      }),
    ),
    http.get(`${base}/rows/:unique_id`, ({ params }) => {
      const number = Number(String(params["unique_id"]).replace(/^nw-/i, ""));
      const found = state.rows.find((entry) => entry.number === number);
      return found ? HttpResponse.json(found) : apiError(404, "not_found", "no such row");
    }),
    http.patch(
      `${base}/views/:view_id`,
      record((body, params) => {
        const found = view(params["view_id"]);
        if (found === undefined) {
          return missing();
        }
        const change = body as ChangeView;
        Object.assign(found, {
          ...(typeof change.name === "string" ? { name: change.name } : {}),
          ...(change.layout ? { layout: change.layout } : {}),
          ...(change.filter === undefined ? {} : { filter: change.filter }),
          ...(change.sorts ? { sorts: change.sorts } : {}),
          ...(change.group === undefined
            ? {}
            : { group: change.group && { order: [], hidden: [], ...change.group } }),
          ...(change.properties ? { properties: change.properties } : {}),
        });
        return HttpResponse.json(found);
      }),
    ),
    http.post(
      `${base}/views`,
      record((body) => {
        const input = body as CreateView;
        const made: View = {
          id: nextId(),
          database_id: DB.tasks,
          position: `b${state.views.length}`,
          name: input.name,
          layout: input.layout,
          sorts: input.sorts ?? [],
          properties: state.properties.map((entry) => ({ property: entry.id, visible: true })),
          ...(input.group ? { group: { order: [], hidden: [], ...input.group } } : {}),
        };
        state.views = [...state.views, made];
        return HttpResponse.json(made, { status: 201 });
      }),
    ),
    http.post(
      `${base}/views/:view_id/duplicate`,
      record((_, params) => {
        const found = view(params["view_id"]);
        if (found === undefined) {
          return missing();
        }
        const made: View = {
          ...structuredClone(found),
          id: nextId(),
          name: `${found.name} (copy)`,
        };
        const at = state.views.indexOf(found) + 1;
        state.views = [...state.views.slice(0, at), made, ...state.views.slice(at)];
        return HttpResponse.json(made, { status: 201 });
      }),
    ),
    http.post(
      `${base}/views/:view_id/move`,
      record((body, params) => {
        const found = view(params["view_id"]);
        if (found === undefined) {
          return missing();
        }
        state.views = placeAmong(state.views, found, (body as Schemas["MoveTo"]).placement);
        return HttpResponse.json(found);
      }),
    ),
    http.delete(
      `${base}/views/:view_id`,
      record((_, params) => {
        state.views = state.views.filter((entry) => entry.id !== params["view_id"]);
        return new HttpResponse(null, { status: 204 });
      }),
    ),
    http.post(
      `${base}/properties`,
      record((body) => {
        const input = body as Schemas["AddProperty"];
        const made: Property = {
          id: nextId(),
          name: input.name,
          type: input.type,
          ...(input.number_format ? { number_format: input.number_format } : {}),
          ...(input.prefix ? { prefix: input.prefix } : {}),
          ...(input.options
            ? {
                options: input.options.map((option) =>
                  Object.assign({}, option, { id: option.id ?? nextId() }),
                ),
              }
            : {}),
        };
        state.properties = [...state.properties, made];
        return HttpResponse.json(made, { status: 201 });
      }),
    ),
    http.patch(
      `${base}/properties/:property_id`,
      record((body, params) => {
        const found = property(params["property_id"]);
        if (found === undefined) {
          return missing();
        }
        const change = body as Schemas["ChangeProperty"];
        if (change.name) found.name = change.name;
        if (change.type) found.type = change.type;
        if (change.number_format) found.number_format = change.number_format;
        if (change.options) {
          found.options = change.options.map((option) => {
            const kept: PropertyOption = { id: option.id ?? nextId(), name: option.name };
            if (option.hue !== undefined && option.hue !== null) kept.hue = option.hue;
            if (option.group) kept.group = option.group;
            return kept;
          });
        }
        return HttpResponse.json(found);
      }),
    ),
    http.post(
      `${base}/properties/:property_id/move`,
      record((body, params) => {
        const found = property(params["property_id"]);
        if (found === undefined) {
          return missing();
        }
        const placement = (body as Schemas["MoveTo"]).placement;
        const rest = state.properties.filter((entry) => entry !== found);
        let at = rest.length;
        if (typeof placement === "object" && "before" in placement) {
          at = rest.findIndex((entry) => entry.id === placement.before);
        } else if (typeof placement === "object" && "after" in placement) {
          at = rest.findIndex((entry) => entry.id === placement.after) + 1;
        }
        state.properties = [...rest.slice(0, at), found, ...rest.slice(at)];
        return HttpResponse.json(found);
      }),
    ),
    http.delete(
      `${base}/properties/:property_id`,
      record((_, params) => {
        state.properties = state.properties.filter((entry) => entry.id !== params["property_id"]);
        return new HttpResponse(null, { status: 204 });
      }),
    ),
    http.get("/api/v1/rows/:id", ({ params }) => {
      const found = state.rows.find((entry) => entry.id === params["id"]);
      return found ? HttpResponse.json(found) : apiError(404, "not_found", "no such row");
    }),
    http.patch(
      "/api/v1/rows/:id",
      record((body, params) => {
        const found = state.rows.find((entry) => entry.id === params["id"]);
        if (found === undefined) {
          return apiError(404, "not_found", "no such row");
        }
        apply(found, (body as { values: Values }).values);
        return HttpResponse.json(found);
      }),
    ),
    http.post(
      "/api/v1/rows/:id/move",
      record((body, params) => {
        const found = state.rows.find((entry) => entry.id === params["id"]);
        if (found === undefined) {
          return apiError(404, "not_found", "no such row");
        }
        const move = body as Schemas["MoveRow"];
        const group = move.group;
        const grouping = group ? property(view(group.view_id)?.group?.property) : undefined;
        if (group && grouping) {
          const to = group.to ?? null;
          if (grouping.type === "person") {
            const kept = keysOf(grouping, found).filter(
              (key) => key !== null && key !== (group.from ?? null),
            );
            const next = to === null ? kept : [...new Set([...kept, to])];
            if (next.length === 0) delete found.values[grouping.id];
            else found.values[grouping.id] = next;
          } else if (to === null) {
            delete found.values[grouping.id];
          } else {
            found.values[grouping.id] = to;
          }
        }
        if (move.placement) {
          state.rows = placeAmong(ordered(), found, move.placement);
        }
        return HttpResponse.json(found);
      }),
    ),
    http.post(
      "/api/v1/workspaces/:id/databases",
      record((body) => {
        const input = body as Schemas["CreateDatabase"];
        const id = nextId();
        const made: Database = {
          page: {
            ...page(
              {
                id,
                title: input.title ?? "",
                ...(input.parent_id ? { parent: input.parent_id } : {}),
                ...(input.teamspace_id ? { teamspace: input.teamspace_id } : {}),
                ...(input.private ? { owner: ada.id } : {}),
              },
              "z",
            ),
            kind: "database",
          },
          inline: input.inline ?? false,
          properties: properties().slice(0, 2),
          views: views()
            .slice(0, 2)
            .map((entry) => Object.assign({}, entry, { database_id: id })),
        };
        created.push(made);
        return HttpResponse.json(made, { status: 201 });
      }),
    ),
  );
  renumber(state.rows);
  return { state, changes, queries, created };
}

type FeedOptions = {
  access?: "edit" | "view";
  /** False to hold `ready` back until the test sends it, as a slow server would. */
  ready?: boolean;
};

/**
 * The server's end of each database feed: it says the connection is ready, as
 * the server does once the connection hears every change, then sends what a
 * test tells it to. The URLs connected to are kept.
 */
export function fakeFeed({ access = "edit", ready = true }: FeedOptions = {}) {
  const urls: string[] = [];
  const readyFor = (url: URL) =>
    JSON.stringify({ type: "ready", database_id: url.pathname.split("/").at(-1) ?? "", access });
  server.use(
    feedLink.addEventListener("connection", ({ client }) => {
      urls.push(client.url.href);
      if (ready) {
        client.send(readyFor(client.url));
      }
    }),
  );
  return {
    urls,
    /** Says every feed is ready, for a feed made with `ready: false`. */
    ready() {
      for (const client of feedLink.clients) {
        client.send(readyFor(client.url));
      }
    },
    /** Tells every feed of a change, as the server does once it is saved. */
    send(event: FeedEvent) {
      feedLink.broadcast(JSON.stringify(event));
    },
    /** Ends every feed with a close code, as the server does. */
    close(code: number) {
      for (const client of feedLink.clients) {
        client.close(code);
      }
    },
  };
}

/** The database's own page. */
export const DATABASE_PATH = `/w/design/p/${DB.tasks}`;

/**
 * What a test gives a database's page, or a row's, to first show: the page,
 * the database and the view's rows are read one after the other, as the
 * page's module loads, which takes a slow machine longer than the second a
 * wait gives by default.
 */
export const FIRST_SHOWN = { timeout: 5000 };
