import { FileText, Lock, Users } from "lucide-react";
import { useId, useMemo, useState } from "react";
import { hasCode } from "@/api/errors";
import { useMovePage, type PagePlace } from "@/api/pages";
import type { WorkspaceSummary } from "@/api/workspaces";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Alert } from "@/components/ui/alert";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { edits } from "@/lib/page-rules";
import { subtreeOf, titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { spaceName, usePlaces } from "./places";

const copy = strings.pages.move;

/** The page to move, with where it is now. */
export type MovedPage = {
  id: string;
  title: string;
  parent_id?: string | null;
  teamspace_id?: string | null;
};

type Destination = {
  key: string;
  label: string;
  detail: string;
  kind: "teamspace" | "private" | "page";
  place: PagePlace;
};

type MoveDialogProps = {
  workspace: WorkspaceSummary;
  page: MovedPage;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onMoved: (title: string) => void;
};

/**
 * Where a page can go, to pick from with the keyboard or a tap: the top of
 * each teamspace the user is in, their private pages, and every page they
 * edit but the page itself and the pages under it, and but databases, which
 * hold only their rows. Picking one moves it there, last among its new
 * siblings.
 */
export function MoveDialog({ workspace, page, open, onOpenChange, onMoved }: MoveDialogProps) {
  const searchId = useId();
  const places = usePlaces(workspace, open);
  const move = useMovePage(workspace.id);
  const [search, setSearch] = useState("");
  const title = titleOf(page);

  const destinations = useMemo(() => {
    const blocked = new Set([page.id]);
    for (const { forest } of places) {
      if (forest.byId.has(page.id)) {
        for (const id of subtreeOf(forest, page.id)) {
          blocked.add(id);
        }
      }
    }
    const atTop = (page.parent_id ?? undefined) === undefined;
    const seen = new Set<string>();
    const list: Destination[] = [];
    for (const { space, forest } of places) {
      if (space.kind === "teamspace" && !(atTop && page.teamspace_id === space.id)) {
        list.push({
          key: `teamspace:${space.id}`,
          label: space.name,
          detail: copy.top(space.name),
          kind: "teamspace",
          place: { teamspace_id: space.id },
        });
      }
      if (space.kind === "private" && !(atTop && (page.teamspace_id ?? undefined) === undefined)) {
        list.push({
          key: "private",
          label: copy.private,
          detail: copy.top(copy.private),
          kind: "private",
          place: { private: true },
        });
      }
      for (const candidate of forest.byId.values()) {
        if (
          seen.has(candidate.id) ||
          blocked.has(candidate.id) ||
          !edits(candidate.access) ||
          candidate.kind !== "page" ||
          candidate.id === page.parent_id
        ) {
          continue;
        }
        seen.add(candidate.id);
        list.push({
          key: candidate.id,
          label: titleOf(candidate),
          detail: copy.in(spaceName(space)),
          kind: "page",
          place: { parent_id: candidate.id },
        });
      }
    }
    return list;
  }, [places, page]);

  const needle = search.trim().toLowerCase();
  const shown =
    needle === ""
      ? destinations
      : destinations.filter((entry) =>
          `${entry.label} ${entry.detail}`.toLowerCase().includes(needle),
        );

  const close = () => {
    setSearch("");
    move.reset();
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? onOpenChange(true) : close())}>
      <DialogContent closeLabel={copy.cancel}>
        <DialogTitle>{copy.title(title)}</DialogTitle>
        <DialogDescription>{copy.lede}</DialogDescription>
        <div className="grid gap-1.5">
          <label htmlFor={searchId} className="text-sm font-medium">
            {copy.search}
          </label>
          <Input
            id={searchId}
            type="search"
            autoComplete="off"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </div>
        {move.isError &&
          (hasCode(move.error, "cycle", "too_deep") ? (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">
                {hasCode(move.error, "cycle") ? copy.cycle : copy.tooDeep}
              </p>
            </Alert>
          ) : (
            <ErrorNotice error={move.error} />
          ))}
        {shown.length === 0 ? (
          <p className="text-sm text-muted-foreground">{copy.none}</p>
        ) : (
          <ul aria-label={copy.places} className="grid max-h-[45dvh] gap-0.5 overflow-y-auto">
            {shown.map((entry) => (
              <li key={entry.key}>
                <button
                  type="button"
                  aria-label={`${entry.label}, ${entry.detail}`}
                  disabled={move.isPending}
                  className="flex min-h-(--tap) w-full items-center gap-2.5 rounded-md px-2.5 text-left text-sm hover:bg-accent disabled:opacity-50"
                  onClick={() =>
                    move.mutate(
                      { id: page.id, ...entry.place, placement: "last" },
                      {
                        onSuccess: () => {
                          close();
                          onMoved(title);
                        },
                      },
                    )
                  }
                >
                  <DestinationIcon kind={entry.kind} />
                  <span className="grid min-w-0 flex-1">
                    <span className="truncate">{entry.label}</span>
                    <span className="truncate text-xs text-muted-foreground">{entry.detail}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </DialogContent>
    </Dialog>
  );
}

function DestinationIcon({ kind }: { kind: Destination["kind"] }) {
  const Icon = kind === "teamspace" ? Users : kind === "private" ? Lock : FileText;
  return <Icon aria-hidden className="size-4 shrink-0 text-muted-foreground" />;
}
