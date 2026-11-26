import { http, HttpResponse } from "msw";
import type { BlockChange, Comparison, Version } from "@/api/history";
import { ada, apiError } from "./fixtures";
import { page, PAGE_IDS } from "./pages";
import { server } from "./server";
import { people, TEAMSPACE_IDS } from "./workspaces";

/** Fixed ids for the versions of the tests. */
export const VERSION_IDS = {
  latest: "0193a5b2-0000-7000-8000-0000000000e1",
  first: "0193a5b2-0000-7000-8000-0000000000e2",
} as const;

/** The version a restore makes. */
const RESTORED_ID = "0193a5b2-0000-7000-8000-0000000000e3";

/** The two versions of Roadmap: Ada wrote it, then Bob changed it. */
export function sampleVersions(): Version[] {
  return [
    {
      id: VERSION_IDS.latest,
      page_id: PAGE_IDS.roadmap,
      created_at: "2026-11-21T10:00:00Z",
      authors: [people.bob.id],
      added: 1,
      changed: 1,
      removed: 0,
    },
    {
      id: VERSION_IDS.first,
      page_id: PAGE_IDS.roadmap,
      created_at: "2026-11-20T09:00:00Z",
      authors: [ada.id],
      added: 2,
      changed: 0,
      removed: 0,
    },
  ];
}

/** A paragraph as BlockNote's JSON has it. */
export function paragraph(id: string, text: string) {
  return {
    id,
    type: "paragraph",
    props: { backgroundColor: "default", textColor: "default", textAlignment: "left" },
    content: [{ type: "text", text, styles: {} }],
    children: [],
  };
}

/** What changed from the first version to now: one block changed, one added. */
export function sampleChanges(): BlockChange[] {
  return [
    { change: "same", depth: 0, block_id: "b1", after: "# Roadmap\n" },
    {
      change: "changed",
      depth: 0,
      block_id: "b2",
      before: "Kick-off in March\n",
      after: "Kick-off in April\n",
    },
    { change: "added", depth: 1, block_id: "b3", after: "Hire a designer\n" },
    { change: "removed", depth: 0, block_id: "b4", before: "Old idea\n" },
  ];
}

type Options = {
  versions?: Version[];
  /** The page's revision now, which a restore must name; another one is refused. */
  revision?: string;
  /** The revision a refused restore moves the page to, as if someone typed meanwhile. */
  changedTo?: string;
  /** The page is as every version has it: comparisons find nothing changed. */
  unchanged?: boolean;
};

/**
 * Roadmap's history: its versions, each read with blocks, compared with the
 * page at its revision, and restored when the restore names that revision.
 * Every restore asked is recorded with its body.
 */
export function fakeHistory(options: Options = {}) {
  const state = {
    versions: options.versions ?? sampleVersions(),
    revision: options.revision ?? "rev-1",
  };
  const restores: { version: string; body: unknown }[] = [];
  let compared = 0;
  const roadmap = page(
    { id: PAGE_IDS.roadmap, title: "Roadmap", teamspace: TEAMSPACE_IDS.general },
    "a0",
  );
  const find = (id: unknown) => state.versions.find((version) => version.id === id);
  const base = "/api/v1/pages/:id/versions";
  server.use(
    http.get(base, () =>
      HttpResponse.json({ page: roadmap, versions: state.versions, next: null }),
    ),
    http.get(`${base}/:version`, ({ params }) => {
      const version = find(params["version"]);
      if (version === undefined) {
        return apiError(404, "not_found", "no such version of this page");
      }
      return HttpResponse.json({
        page: roadmap,
        version,
        blocks: [paragraph("b2", "Kick-off in March")],
        markdown: "Kick-off in March\n",
      });
    }),
    http.get(`${base}/:version/compare`, ({ params }) => {
      const version = find(params["version"]);
      if (version === undefined) {
        return apiError(404, "not_found", "no such version of this page");
      }
      compared += 1;
      const comparison: Comparison = options.unchanged
        ? {
            page: roadmap,
            version,
            changes: [{ change: "same", depth: 0, block_id: "b1", after: "# Roadmap\n" }],
            added: 0,
            changed: 0,
            removed: 0,
            revision: state.revision,
          }
        : {
            page: roadmap,
            version,
            changes: sampleChanges(),
            added: 1,
            changed: 1,
            removed: 1,
            revision: state.revision,
          };
      return HttpResponse.json(comparison);
    }),
    http.post(`${base}/:version/restore`, async ({ request, params }) => {
      const body = (await request.json()) as { base: string };
      restores.push({ version: String(params["version"]), body });
      const version = find(params["version"]);
      if (version === undefined) {
        return apiError(404, "not_found", "no such version of this page");
      }
      if (body.base !== state.revision) {
        return apiError(409, "edit_conflict", "the page changed since that revision was read");
      }
      if (options.changedTo !== undefined && state.revision !== options.changedTo) {
        // Someone typed meanwhile: this restore is refused, and the page moves on.
        state.revision = options.changedTo;
        return apiError(409, "edit_conflict", "the page changed since that revision was read");
      }
      state.revision = "rev-restored";
      // As the server does: the page as it was, then the restore, become the latest versions.
      state.versions = [
        {
          id: RESTORED_ID,
          page_id: PAGE_IDS.roadmap,
          created_at: "2026-11-22T10:00:00Z",
          authors: [ada.id],
          added: 0,
          changed: 1,
          removed: 1,
          restored_from: version.id,
          restored_from_at: version.created_at,
        },
        ...state.versions,
      ];
      return HttpResponse.json({ page: roadmap, version, revision: state.revision });
    }),
  );
  return { restores, compared: () => compared };
}
