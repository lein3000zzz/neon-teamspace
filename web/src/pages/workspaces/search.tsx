import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { Search } from "lucide-react";
import { useEffect, useId, useState } from "react";
import type { PageKind } from "@/api/pages";
import { searchQuery, type SearchFilters, type SearchResult } from "@/api/search";
import { teamspacesQuery } from "@/api/teamspaces";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { PersonLabel } from "@/components/people/person-label";
import { useWorkspacePeople } from "@/components/people/use-people";
import { PageGlyph } from "@/components/pages/page-tree";
import { Runs } from "@/components/search/runs";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { useDebounced } from "@/lib/debounce";
import { formatDateTime } from "@/lib/format";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { seesTeamspaces } from "@/lib/workspace-rules";

const copy = strings.search.page;

/** How long typing pauses before the search runs and the address follows. */
const TYPING_PAUSE_MS = 250;

const KINDS = ["page", "database", "row"] as const satisfies readonly PageKind[];
const WITHINS = ["1d", "7d", "30d", "365d"] as const;
type Within = (typeof WITHINS)[number];

/** What the address says the search is: its query and filters. */
export type SearchPageSearch = {
  q?: string;
  teamspace?: string;
  kind?: PageKind;
  /** Created by. */
  by?: string;
  /** Last edited by. */
  editor?: string;
  within?: Within;
};

const text = (value: unknown) => (typeof value === "string" && value !== "" ? value : undefined);

/** A select's value, or undefined for its first option, which is none. */
const pick = (value: string) => (value === "" ? undefined : value);

/** Reads the address's search, keeping only what makes sense. */
export function readSearch(search: Record<string, unknown>): SearchPageSearch {
  const kind = KINDS.find((entry) => entry === search["kind"]);
  const within = WITHINS.find((entry) => entry === search["within"]);
  const parts: SearchPageSearch = {};
  const q = text(search["q"]);
  const teamspace = text(search["teamspace"]);
  const by = text(search["by"]);
  const editor = text(search["editor"]);
  if (q !== undefined) parts.q = q;
  if (teamspace !== undefined) parts.teamspace = teamspace;
  if (kind !== undefined) parts.kind = kind;
  if (by !== undefined) parts.by = by;
  if (editor !== undefined) parts.editor = editor;
  if (within !== undefined) parts.within = within;
  return parts;
}

function filtersOf(search: SearchPageSearch): SearchFilters {
  return {
    ...(search.teamspace === undefined ? {} : { teamspace_id: search.teamspace }),
    ...(search.kind === undefined ? {} : { kind: search.kind }),
    ...(search.by === undefined ? {} : { created_by: search.by }),
    ...(search.editor === undefined ? {} : { edited_by: search.editor }),
    ...(search.within === undefined ? {} : { edited_within: search.within }),
  };
}

/**
 * The workspace's search: a query, filters by teamspace, kind, who made or
 * last changed a page and how recently, and the results the best first, a
 * part at a time. Everything is in the address, so a search can be shared and
 * the back button undoes a filter. Without a query, the filters list what
 * changed last. Guests, who know no other member, filter by no one.
 */
export function SearchPage() {
  const workspace = useWorkspace();
  const search = useSearch({ from: "/_app/w/$slug/search" });
  const navigate = useNavigate({ from: "/w/$slug/search" });
  const fieldId = useId();
  const [typing, setTyping] = useState(search.q ?? "");
  const typed = useDebounced(typing.trim(), TYPING_PAUSE_MS);
  const member = seesTeamspaces(workspace.role);

  useEffect(() => {
    if (typed !== (search.q ?? "")) {
      void navigate({
        search: (previous) => readSearch({ ...previous, q: typed }),
        replace: true,
      });
    }
  }, [typed, search.q, navigate]);

  const results = useInfiniteQuery(searchQuery(workspace.id, search.q ?? "", filtersOf(search)));
  const found = results.data?.pages.flatMap((part) => part.results) ?? [];
  const set = (change: Change) =>
    void navigate({ search: (previous) => readSearch({ ...previous, ...change }) });

  return (
    <div className="grid gap-6">
      <PageHeader
        eyebrow={`${copy.eyebrow} · ${workspace.name}`}
        title={copy.title}
        lede={copy.lede}
      >
        <search className="mt-3 flex items-center gap-2">
          <Search aria-hidden className="size-4 shrink-0 text-muted-foreground" />
          <label htmlFor={fieldId} className="sr-only">
            {copy.label}
          </label>
          <input
            id={fieldId}
            type="search"
            value={typing}
            placeholder={copy.placeholder}
            onChange={(event) => setTyping(event.target.value)}
            className="h-(--tap) min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-base outline-none focus-visible:border-primary md:text-sm"
          />
        </search>
      </PageHeader>

      <Filters search={search} member={member} onChange={set} />

      <section aria-label={search.q ? copy.results : copy.recent} className="grid gap-3">
        {results.isPending && <Loading />}
        {results.isError && (
          <ErrorNotice
            error={results.error}
            retrying={results.isFetching}
            onRetry={() => void results.refetch()}
          />
        )}
        {results.isSuccess && found.length === 0 && (
          <p className="text-sm text-muted-foreground">{copy.none}</p>
        )}
        {found.length > 0 && (
          <ul className="grid gap-2">
            {found.map((result) => (
              <Result key={result.page.id} result={result} />
            ))}
          </ul>
        )}
        {results.hasNextPage && (
          <Button
            variant="outline"
            className="justify-self-start"
            disabled={results.isFetchingNextPage}
            onClick={() => void results.fetchNextPage()}
          >
            {results.isFetchingNextPage ? copy.loadingMore : copy.more}
          </Button>
        )}
      </section>
    </div>
  );
}

/** A filter set, or cleared. */
type Change = { [Key in keyof SearchPageSearch]?: SearchPageSearch[Key] | undefined };

type FiltersProps = {
  search: SearchPageSearch;
  /** Not a guest: teamspaces and people to filter by. */
  member: boolean;
  onChange: (change: Change) => void;
};

function Filters({ search, member, onChange }: FiltersProps) {
  const workspace = useWorkspace();
  const ids = { teamspace: useId(), kind: useId(), by: useId(), editor: useId(), within: useId() };
  const teamspaces = useQuery({ ...teamspacesQuery(workspace.id), enabled: member });
  const people = useWorkspacePeople(workspace.id, member);
  const everyone = [...people.values()].toSorted((a, b) =>
    a.display_name.localeCompare(b.display_name),
  );

  return (
    <section aria-label={copy.filters} className="grid gap-3 sm:grid-cols-3 lg:grid-cols-5">
      {member && (
        <div className="grid gap-1.5">
          <Label htmlFor={ids.teamspace}>{copy.teamspace}</Label>
          <NativeSelect
            id={ids.teamspace}
            value={search.teamspace ?? ""}
            onChange={(event) => onChange({ teamspace: pick(event.target.value) })}
          >
            <option value="">{copy.anyTeamspace}</option>
            {(teamspaces.data?.teamspaces ?? []).map((teamspace) => (
              <option key={teamspace.id} value={teamspace.id}>
                {teamspace.name}
              </option>
            ))}
          </NativeSelect>
        </div>
      )}
      <div className="grid gap-1.5">
        <Label htmlFor={ids.kind}>{copy.kind}</Label>
        <NativeSelect
          id={ids.kind}
          value={search.kind ?? ""}
          onChange={(event) =>
            onChange({ kind: KINDS.find((kind) => kind === event.target.value) })
          }
        >
          <option value="">{copy.anyKind}</option>
          {KINDS.map((kind) => (
            <option key={kind} value={kind}>
              {copy.kinds[kind]}
            </option>
          ))}
        </NativeSelect>
      </div>
      {member && (
        <>
          <div className="grid gap-1.5">
            <Label htmlFor={ids.by}>{copy.createdBy}</Label>
            <NativeSelect
              id={ids.by}
              value={search.by ?? ""}
              onChange={(event) => onChange({ by: pick(event.target.value) })}
            >
              <option value="">{copy.anyone}</option>
              {everyone.map((person) => (
                <option key={person.id} value={person.id}>
                  {person.display_name}
                </option>
              ))}
            </NativeSelect>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor={ids.editor}>{copy.editedBy}</Label>
            <NativeSelect
              id={ids.editor}
              value={search.editor ?? ""}
              onChange={(event) => onChange({ editor: pick(event.target.value) })}
            >
              <option value="">{copy.anyone}</option>
              {everyone.map((person) => (
                <option key={person.id} value={person.id}>
                  {person.display_name}
                </option>
              ))}
            </NativeSelect>
          </div>
        </>
      )}
      <div className="grid gap-1.5">
        <Label htmlFor={ids.within}>{copy.within}</Label>
        <NativeSelect
          id={ids.within}
          value={search.within ?? ""}
          onChange={(event) =>
            onChange({ within: WITHINS.find((within) => within === event.target.value) })
          }
        >
          <option value="">{copy.anyTime}</option>
          {WITHINS.map((within) => (
            <option key={within} value={within}>
              {copy.withins[within]}
            </option>
          ))}
        </NativeSelect>
      </div>
    </section>
  );
}

function Result({ result }: { result: SearchResult }) {
  const workspace = useWorkspace();
  const people = useWorkspacePeople(workspace.id, seesTeamspaces(workspace.role));
  const page = result.page;
  const edited =
    page.updated_at > page.content_updated_at ? page.updated_at : page.content_updated_at;
  const editor = page.last_edited_by ?? null;
  return (
    <li className="grid gap-1 rounded-lg border border-border bg-card p-4">
      <Link
        to="/w/$slug/p/$pageId"
        params={{ slug: workspace.slug, pageId: page.id }}
        className="flex min-w-0 items-center gap-2 font-medium hover:underline"
      >
        <PageGlyph icon={page.icon} kind={page.kind} />
        <span className="truncate">
          {result.title.length > 0 ? <Runs runs={result.title} /> : titleOf(page)}
        </span>
      </Link>
      {result.snippet.length > 0 && (
        <p className="line-clamp-2 text-sm text-muted-foreground">
          <Runs runs={result.snippet} />
        </p>
      )}
      <p className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
        <span>{strings.search.kinds[page.kind]}</span>
        <span aria-hidden>·</span>
        <span>{copy.edited(formatDateTime(edited))}</span>
        {editor !== null && (
          <>
            <span aria-hidden>·</span>
            <PersonLabel person={people.get(editor)} />
          </>
        )}
      </p>
    </li>
  );
}
