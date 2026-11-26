import { http, HttpResponse, type HttpResponseResolver } from "msw";
import type { Schemas } from "@/api/client";
import type { Crumb, Page, PageAccess, Placement, Share, TreePage } from "@/api/pages";
import { ada, apiError } from "./fixtures";
import { server } from "./server";
import { asPerson, people, scout, TEAMSPACE_IDS, type Recorded } from "./workspaces";

const WORKSPACE_ID = "0193a5b2-0000-7000-8000-00000000a001";
const CREATED = "2026-11-20T09:00:00Z";
const LATER = "2026-11-21T09:00:00Z";

/** Fixed ids for the pages of the tests, and for those they make, in order. */
export const PAGE_IDS = {
  roadmap: "0193a5b2-0000-7000-8000-0000000000a1",
  q1: "0193a5b2-0000-7000-8000-0000000000a2",
  goals: "0193a5b2-0000-7000-8000-0000000000a3",
  launch: "0193a5b2-0000-7000-8000-0000000000a4",
  notes: "0193a5b2-0000-7000-8000-0000000000a5",
  old: "0193a5b2-0000-7000-8000-0000000000a6",
  blank: "0193a5b2-0000-7000-8000-0000000000a7",
} as const;

/** The ids of the files uploaded, in turn. */
export const UPLOADED_IDS = [
  "0193a5b2-0000-7000-8000-0000000000f1",
  "0193a5b2-0000-7000-8000-0000000000f2",
  "0193a5b2-0000-7000-8000-0000000000f3",
] as const;

const MADE_IDS = [
  "0193a5b2-0000-7000-8000-0000000000b1",
  "0193a5b2-0000-7000-8000-0000000000b2",
  "0193a5b2-0000-7000-8000-0000000000b3",
] as const;

type Seed = {
  id: string;
  title: string;
  parent?: string;
  teamspace?: string;
  owner?: string;
  access?: PageAccess;
  icon?: string;
  trashed?: boolean;
};

/** A page as the server sends it, with the defaults of a new one. */
export function page(seed: Seed, position: string): Page {
  return {
    id: seed.id,
    workspace_id: WORKSPACE_ID,
    kind: "page",
    title: seed.title,
    position,
    style: { font: "default", small_text: false, full_width: false },
    access: seed.access ?? "edit",
    created_by: ada.id,
    last_edited_by: ada.id,
    created_at: CREATED,
    updated_at: CREATED,
    content_updated_at: CREATED,
    ...(seed.parent === undefined ? {} : { parent_id: seed.parent }),
    ...(seed.teamspace === undefined ? {} : { teamspace_id: seed.teamspace }),
    ...(seed.owner === undefined ? {} : { owner_id: seed.owner }),
    ...(seed.icon === undefined ? {} : { icon: seed.icon }),
    ...(seed.trashed ? { trashed_at: LATER, trashed_by: ada.id, trashed_with: seed.id } : {}),
  };
}

/**
 * General's pages, Roadmap > Q1 > Goals and Launch beside Roadmap, and Notes,
 * one of Ada's private pages. Old is in the trash.
 */
export function samplePages(): Page[] {
  const general = TEAMSPACE_IDS.general;
  return [
    page({ id: PAGE_IDS.roadmap, title: "Roadmap", teamspace: general, icon: "🗺️" }, "a0"),
    page({ id: PAGE_IDS.q1, title: "Q1", teamspace: general, parent: PAGE_IDS.roadmap }, "a0"),
    page({ id: PAGE_IDS.goals, title: "Goals", teamspace: general, parent: PAGE_IDS.q1 }, "a0"),
    page({ id: PAGE_IDS.launch, title: "Launch", teamspace: general }, "a1"),
    page({ id: PAGE_IDS.notes, title: "Notes", owner: ada.id }, "a0"),
    page({ id: PAGE_IDS.old, title: "Old plans", teamspace: general, trashed: true }, "a2"),
  ];
}

type Options = {
  pages?: Page[];
  favorites?: string[];
  /** The shares set on each page, by its id. */
  shares?: Record<string, Share[]>;
  /** What the shared-pages tree answers. */
  shared?: TreePage[];
};

const isLive = (entry: Page) => (entry.trashed_at ?? null) === null;
const missing = () => apiError(404, "not_found", "no such page");

/** An export's answer: its content, named as the server names it. */
const exported = (body: string, type: string, name: string) =>
  new HttpResponse(body, {
    headers: {
      "Content-Type": type,
      "Content-Disposition": `attachment; filename="${name}"; filename*=UTF-8''${encodeURIComponent(name)}`,
    },
  });

/**
 * The page routes of the server for Design, holding what changes, so a page
 * reads back what it did. Every request that changes something is recorded.
 * Call after fakeWorkspaces(), whose routes these sit beside.
 */
export function fakePages(options: Options = {}) {
  const state = {
    pages: options.pages ?? samplePages(),
    favorites: options.favorites ?? [],
    shares: options.shares ?? ({} as Record<string, Share[]>),
    shared: options.shared ?? [],
  };
  const changes: Recorded[] = [];
  /** Each import, with where it went and the file it carried. */
  const imports: { query: Record<string, string>; text: string }[] = [];
  /** Each export asked for, by its path and query. */
  const exports: string[] = [];
  /** Each file uploaded, with the page and kind it was for and its name. */
  const uploads: { page: string; kind: string; name: string }[] = [];
  let made = 0;

  const record = (
    resolver: (body: unknown, params: Record<string, string>) => Response | Promise<Response>,
  ) =>
    (async ({ request, params }) => {
      const text = await request.text();
      const body: unknown = text === "" ? undefined : JSON.parse(text);
      changes.push({ method: request.method, path: new URL(request.url).pathname, body });
      return resolver(body, params as Record<string, string>);
    }) satisfies HttpResponseResolver;

  const find = (id: string | undefined) => state.pages.find((entry) => entry.id === id);
  const childrenOf = (parent: string | undefined, space: (entry: Page) => boolean) =>
    state.pages
      .filter((entry) => isLive(entry) && space(entry) && (entry.parent_id ?? undefined) === parent)
      .toSorted((a, b) => (a.position < b.position ? -1 : a.position > b.position ? 1 : 0));
  const subtree = (id: string): Page[] => {
    const root = find(id);
    if (root === undefined) {
      return [];
    }
    return [
      root,
      ...state.pages
        .filter((entry) => entry.parent_id === id)
        .flatMap((entry) => subtree(entry.id)),
    ];
  };
  const treeEntry = (entry: Page, space: (entry: Page) => boolean): TreePage => ({
    id: entry.id,
    kind: entry.kind,
    title: entry.title,
    access: entry.access,
    child_count: childrenOf(entry.id, space).length,
    ...(entry.parent_id ? { parent_id: entry.parent_id } : {}),
    ...(entry.teamspace_id ? { teamspace_id: entry.teamspace_id } : {}),
    ...(entry.owner_id ? { owner_id: entry.owner_id } : {}),
    ...(entry.icon ? { icon: entry.icon } : {}),
  });
  /** A space's pages, each before the pages under it, as the server lists trees. */
  const tree = (space: (entry: Page) => boolean): TreePage[] => {
    const listed: TreePage[] = [];
    const walk = (parent: string | undefined) => {
      for (const entry of childrenOf(parent, space)) {
        listed.push(treeEntry(entry, space));
        walk(entry.id);
      }
    };
    walk(undefined);
    return listed;
  };

  type Place = Schemas["MovePage"];
  /** Moves `target` to the place, its siblings renumbered in their new order. */
  const place = (target: Page, where: Place) => {
    const parent = find(where.parent_id ?? undefined);
    const teamspace = parent ? parent.teamspace_id : (where.teamspace_id ?? undefined);
    const owner = parent ? parent.owner_id : where.private ? ada.id : undefined;
    const space = (entry: Page) =>
      teamspace ? entry.teamspace_id === teamspace : entry.owner_id === owner;
    const siblings = childrenOf(parent?.id, space).filter((entry) => entry.id !== target.id);
    const placement: Placement = where.placement ?? "last";
    let at = siblings.length;
    if (placement === "first") {
      at = 0;
    } else if (typeof placement === "object" && "before" in placement) {
      at = siblings.findIndex((entry) => entry.id === placement.before);
    } else if (typeof placement === "object" && "after" in placement) {
      at = siblings.findIndex((entry) => entry.id === placement.after) + 1;
    }
    const moved: Page = { ...target, updated_at: LATER };
    delete moved.parent_id;
    delete moved.teamspace_id;
    delete moved.owner_id;
    if (parent) moved.parent_id = parent.id;
    if (teamspace) moved.teamspace_id = teamspace;
    if (owner) moved.owner_id = owner;
    for (const entry of subtree(target.id).slice(1)) {
      Object.assign(entry, {
        teamspace_id: teamspace,
        owner_id: owner,
      });
    }
    const order = [...siblings.slice(0, at), moved, ...siblings.slice(at)];
    order.forEach((entry, index) => {
      entry.position = `a${index}`;
    });
    state.pages = [...state.pages.filter((entry) => entry.id !== target.id), moved];
    return moved;
  };

  const base = "/api/v1/pages/:id";
  server.use(
    // One page per import, titled after the file's first heading, as the server titles
    // them; a zip, of that type, also leaves out an image. The file's name is not kept:
    // jsdom's files lose theirs on their way into Node's requests.
    http.post("/api/v1/workspaces/:id/pages/import", async ({ request }) => {
      const query = Object.fromEntries(new URL(request.url).searchParams);
      const file = (await request.formData()).get("file");
      const text = typeof file === "string" || file === null ? "" : await file.text();
      const zip = typeof file !== "string" && file?.type === "application/zip";
      imports.push({ query, text });
      const id = MADE_IDS[made] ?? MADE_IDS[0];
      made += 1;
      const title = /^# (.+)$/m.exec(text)?.[1] ?? "Imported";
      const created = page({ id, title }, "a0");
      state.pages.push(created);
      const placed = place(created, {
        ...(query["parent_id"] ? { parent_id: query["parent_id"] } : {}),
        ...(query["teamspace_id"] ? { teamspace_id: query["teamspace_id"] } : {}),
        ...(query["private"] === "true" ? { private: true } : {}),
      });
      return HttpResponse.json(
        {
          pages: [placed],
          count: 1,
          skipped: zip ? [{ path: "Roadmap/logo.png", reason: "attachment" }] : [],
        },
        { status: 201 },
      );
    }),
    http.get(`${base}/export`, ({ request, params }) => {
      const url = new URL(request.url);
      exports.push(`${url.pathname}${url.search}`);
      const found = find(String(params["id"]));
      if (found === undefined) {
        return missing();
      }
      return url.searchParams.get("children") === "true"
        ? exported("PK", "application/zip", `${found.title}.zip`)
        : exported(`# ${found.title}\n`, "text/markdown; charset=utf-8", `${found.title}.md`);
    }),
    http.get("/api/v1/teamspaces/:id/export", ({ request }) => {
      exports.push(new URL(request.url).pathname);
      return exported("PK", "application/zip", "General.zip");
    }),
    http.get("/api/v1/teamspaces/:id/pages", ({ params }) =>
      HttpResponse.json({ pages: tree((entry) => entry.teamspace_id === params["id"]) }),
    ),
    http.get("/api/v1/workspaces/:id/private-pages", () =>
      HttpResponse.json({ pages: tree((entry) => entry.owner_id === ada.id) }),
    ),
    http.get("/api/v1/workspaces/:id/shared-pages", () =>
      HttpResponse.json({ pages: state.shared }),
    ),
    http.get("/api/v1/workspaces/:id/trash", () =>
      HttpResponse.json({
        pages: state.pages.filter((entry) => !isLive(entry) && entry.trashed_with === entry.id),
      }),
    ),
    http.post(
      "/api/v1/workspaces/:id/pages",
      record((body) => {
        const input = body as Schemas["CreatePage"];
        const id = MADE_IDS[made] ?? MADE_IDS[0];
        made += 1;
        const created = page({ id, title: input.title ?? "" }, "a0");
        state.pages.push(created);
        return HttpResponse.json(
          place(created, {
            ...(input.parent_id ? { parent_id: input.parent_id } : {}),
            ...(input.teamspace_id ? { teamspace_id: input.teamspace_id } : {}),
            ...(input.private ? { private: true } : {}),
            ...(input.placement ? { placement: input.placement } : {}),
          }),
          { status: 201 },
        );
      }),
    ),
    http.get(base, ({ params }) => {
      const found = find(String(params["id"]));
      return found ? HttpResponse.json(found) : missing();
    }),
    http.patch(
      base,
      record((body, params) => {
        const found = find(params["id"]);
        if (found === undefined) {
          return missing();
        }
        const patch = body as Schemas["UpdatePage"];
        if (typeof patch.title === "string") found.title = patch.title.trim();
        if (patch.icon !== undefined) found.icon = patch.icon;
        // One cover at a time, as the server keeps them.
        if (patch.cover !== undefined) {
          found.cover = patch.cover;
          if (patch.cover !== null) found.cover_image = null;
        }
        if (patch.cover_image !== undefined) {
          found.cover_image = patch.cover_image;
          if (patch.cover_image !== null) found.cover = null;
        }
        if (patch.style) {
          found.style = {
            font: patch.style.font ?? found.style.font,
            small_text: patch.style.small_text ?? found.style.small_text,
            full_width: patch.style.full_width ?? found.style.full_width,
          };
        }
        return HttpResponse.json(found);
      }),
    ),
    // A file uploaded to a page, as the server answers it. jsdom's files lose their names on
    // their way into Node's requests, so the name is the form's own.
    http.post(`${base}/files`, async ({ request, params }) => {
      const kind = (new URL(request.url).searchParams.get("kind") ?? "file") as Schemas["FileKind"];
      const file = (await request.formData()).get("file");
      const size = typeof file === "string" || file === null ? 0 : file.size;
      const id = UPLOADED_IDS[uploads.length] ?? UPLOADED_IDS[0];
      uploads.push({ page: String(params["id"]), kind, name: "upload" });
      return HttpResponse.json(
        {
          id,
          page_id: String(params["id"]),
          url: `/api/v1/files/${id}`,
          kind,
          name: "upload",
          content_type: "image/png",
          size,
          sha256: "0".repeat(64),
          width: null,
          height: null,
          created_at: CREATED,
        } satisfies Schemas["File"],
        { status: 201 },
      );
    }),
    http.post(
      `${base}/move`,
      record((body, params) => {
        const found = find(params["id"]);
        return found ? HttpResponse.json(place(found, body as Place)) : missing();
      }),
    ),
    http.post(
      `${base}/trash`,
      record((_, params) => {
        const found = find(params["id"]);
        if (found === undefined) {
          return missing();
        }
        for (const entry of subtree(found.id).filter(isLive)) {
          Object.assign(entry, { trashed_at: LATER, trashed_by: ada.id, trashed_with: found.id });
        }
        return HttpResponse.json(found);
      }),
    ),
    http.post(
      `${base}/restore`,
      record((_, params) => {
        const found = find(params["id"]);
        if (found === undefined) {
          return missing();
        }
        for (const entry of state.pages.filter((item) => item.trashed_with === found.id)) {
          delete entry.trashed_at;
          delete entry.trashed_by;
          delete entry.trashed_with;
        }
        return HttpResponse.json(found);
      }),
    ),
    http.delete(
      base,
      record((_, params) => {
        const gone = new Set(subtree(String(params["id"])).map((entry) => entry.id));
        state.pages = state.pages.filter((entry) => !gone.has(entry.id));
        return new HttpResponse(null, { status: 204 });
      }),
    ),
    http.get(`${base}/breadcrumbs`, ({ params }) => {
      const crumbs: Crumb[] = [];
      for (let at = find(String(params["id"])); at; at = find(at.parent_id ?? undefined)) {
        crumbs.unshift({ id: at.id, title: at.title, ...(at.icon ? { icon: at.icon } : {}) });
      }
      return crumbs.length === 0 ? missing() : HttpResponse.json({ pages: crumbs });
    }),
    http.get(`${base}/shares`, ({ params }) => {
      const own = state.shares[String(params["id"])] ?? [];
      const inherited: Share[] = [];
      for (
        let at = find(find(String(params["id"]))?.parent_id ?? undefined);
        at;
        at = find(at.parent_id ?? undefined)
      ) {
        for (const share of state.shares[at.id] ?? []) {
          inherited.push(Object.assign(structuredClone(share), { inherited: true }));
        }
      }
      return HttpResponse.json({ shares: [...own, ...inherited] });
    }),
    http.post(
      `${base}/shares`,
      record((body, params) => {
        const input = body as Schemas["CreateShare"];
        const id = String(params["id"]);
        const person = [...Object.values(people), asPerson(scout)].find(
          (entry) => entry.id === input.user_id,
        );
        if (person === undefined) {
          return apiError(404, "not_found", "no such member");
        }
        const list = state.shares[id] ?? [];
        if (list.some((share) => share.user.id === person.id)) {
          return apiError(409, "already_shared", "the page is shared with them already");
        }
        const share: Share = {
          user: person,
          access: input.access,
          page_id: id,
          inherited: false,
          created_at: LATER,
          updated_at: LATER,
        };
        state.shares[id] = [...list, share];
        return HttpResponse.json(share, { status: 201 });
      }),
    ),
    http.patch(
      `${base}/shares/:user_id`,
      record((body, params) => {
        const share = (state.shares[String(params["id"])] ?? []).find(
          (entry) => entry.user.id === params["user_id"],
        );
        if (share === undefined) {
          return apiError(404, "not_found", "no such share");
        }
        share.access = (body as Schemas["ChangeShare"]).access;
        return HttpResponse.json(share);
      }),
    ),
    http.delete(
      `${base}/shares/:user_id`,
      record((_, params) => {
        const id = String(params["id"]);
        state.shares[id] = (state.shares[id] ?? []).filter(
          (entry) => entry.user.id !== params["user_id"],
        );
        return new HttpResponse(null, { status: 204 });
      }),
    ),
    http.get("/api/v1/account/favorites", () =>
      HttpResponse.json({
        pages: state.favorites
          .map((id) => find(id))
          .filter((entry): entry is Page => entry !== undefined && isLive(entry)),
      }),
    ),
    http.post(
      "/api/v1/account/favorites",
      record((body) => {
        const id = (body as Schemas["AddFavorite"]).page_id;
        const found = find(id);
        if (found === undefined) {
          return missing();
        }
        state.favorites = [...state.favorites, id];
        return HttpResponse.json(found, { status: 201 });
      }),
    ),
    http.delete(
      "/api/v1/account/favorites/:page_id",
      record((_, params) => {
        state.favorites = state.favorites.filter((id) => id !== params["page_id"]);
        return new HttpResponse(null, { status: 204 });
      }),
    ),
  );
  return { state, changes, imports, exports, uploads, find };
}
