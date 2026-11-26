import { http, HttpResponse } from "msw";
import type { Page } from "@/api/pages";
import type { SearchResult } from "@/api/search";
import { samplePages } from "./pages";
import { server } from "./server";

/** A result for a page: its title marked where it starts with `marked`, and a snippet. */
export function result(page: Page, marked: string, snippet: string): SearchResult {
  const at = page.title.toLowerCase().indexOf(marked.toLowerCase());
  const title =
    at < 0
      ? [{ text: page.title, highlight: false }]
      : [
          { text: page.title.slice(0, at), highlight: false },
          { text: page.title.slice(at, at + marked.length), highlight: true },
          { text: page.title.slice(at + marked.length), highlight: false },
        ].filter((run) => run.text !== "");
  return {
    page,
    title,
    snippet: snippet === "" ? [] : [{ text: snippet, highlight: true }],
    rank: 1,
  };
}

type Options = {
  /** What a query finds, by the query; the rest find nothing. */
  answers?: Record<string, SearchResult[]>;
  /** The pages changed last. */
  recent?: Page[];
  /** Results per part on the search page, the next part after `next`. */
  part?: number;
};

/**
 * The search routes of the server for Design: each query finds what `answers`
 * says, a part at a time, and the recent pages are General's. Every search
 * asked is recorded with its parameters.
 */
export function fakeSearch(options: Options = {}) {
  const asked: Record<string, string>[] = [];
  server.use(
    http.get("/api/v1/workspaces/:id/search", ({ request }) => {
      const params = Object.fromEntries(new URL(request.url).searchParams);
      asked.push(params);
      const all = options.answers?.[params["query"] ?? ""] ?? [];
      const from = Number(params["after"] ?? "0");
      const part = options.part ?? Number(params["limit"] ?? "20");
      const results = all.slice(from, from + part);
      const next = from + part < all.length ? String(from + part) : null;
      return HttpResponse.json({ results, next });
    }),
    http.get("/api/v1/workspaces/:id/recent-pages", () =>
      HttpResponse.json({
        pages: options.recent ?? samplePages().filter((page) => page.trashed_at === undefined),
      }),
    ),
  );
  return { asked };
}
