import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import {
  ArrowDown,
  ArrowUp,
  Download,
  FolderInput,
  MoreHorizontal,
  Pencil,
  Plus,
  Star,
  StarOff,
  Trash2,
  Upload,
  UserPlus,
} from "lucide-react";
import { useState } from "react";
import {
  favoritesQuery,
  useAddFavorite,
  useExportPage,
  useRemoveFavorite,
  useTrashPage,
  type PageAccess,
} from "@/api/pages";
import type { WorkspaceSummary } from "@/api/workspaces";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { edits, mayShare } from "@/lib/page-rules";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { ImportDialog } from "./import-dialog";
import { MoveDialog, type MovedPage } from "./move-dialog";
import { usePageNav } from "./page-nav-state";
import { ShareDialog } from "./share-dialog";

const copy = strings.pages.menu;

type PageMenuProps = {
  workspace: WorkspaceSummary;
  page: MovedPage & { access: PageAccess };
  /** For the trigger's look beside the tree row or the page's title. */
  className?: string;
  /** Present where the title can be renamed in place, as in the tree. */
  onRename?: (() => void) | undefined;
  onAddInside?: (() => void) | undefined;
  /** Present when there is a sibling to pass, the keyboard's way to reorder. */
  onMoveUp?: (() => void) | undefined;
  onMoveDown?: (() => void) | undefined;
  /** Runs as a page made here is opened, such as to close the drawer. */
  onNavigate?: (() => void) | undefined;
};

/**
 * Everything to do with a page from one button: rename, add a page inside,
 * import markdown into it, export it, move it, favorite it, share it, or move
 * it to the trash. Each shows only to those it can work for: a viewer gets the
 * favorite and the exports.
 */
export function PageMenu({
  workspace,
  page,
  className,
  onRename,
  onAddInside,
  onMoveUp,
  onMoveDown,
  onNavigate,
}: PageMenuProps) {
  const nav = usePageNav();
  const favorites = useQuery(favoritesQuery(workspace.id));
  const addFavorite = useAddFavorite(workspace.id);
  const removeFavorite = useRemoveFavorite(workspace.id);
  const trash = useTrashPage(workspace.id);
  const report = (error: unknown) => nav.setNotice({ tone: "error", error });
  const exporter = useExportPage({ onSaved: () => nav.setNotice(null), onFailed: report });
  const [dialog, setDialog] = useState<"move" | "share" | "import" | null>(null);
  const title = titleOf(page);
  const editor = edits(page.access);
  const favorite = favorites.data?.some((entry) => entry.id === page.id) ?? false;
  const exportAs = (children: boolean) => {
    nav.setNotice({ tone: "info", text: strings.pages.transfer.exporting(title) });
    exporter.mutate({ id: page.id, children });
  };

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={strings.pages.tree.more(title)}
            className={cn("text-muted-foreground", className)}
          >
            <MoreHorizontal aria-hidden />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          {editor && onRename && (
            <DropdownMenuItem onSelect={onRename}>
              <Pencil aria-hidden />
              {copy.rename}
            </DropdownMenuItem>
          )}
          {editor && onAddInside && (
            <DropdownMenuItem onSelect={onAddInside}>
              <Plus aria-hidden />
              {copy.addInside}
            </DropdownMenuItem>
          )}
          {favorite ? (
            <DropdownMenuItem onSelect={() => removeFavorite.mutate(page.id, { onError: report })}>
              <StarOff aria-hidden />
              {copy.unfavorite}
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem onSelect={() => addFavorite.mutate(page.id, { onError: report })}>
              <Star aria-hidden />
              {copy.favorite}
            </DropdownMenuItem>
          )}
          {mayShare(workspace.role, page.access) && (
            <DropdownMenuItem onSelect={() => setDialog("share")}>
              <UserPlus aria-hidden />
              {copy.share}
            </DropdownMenuItem>
          )}
          <DropdownMenuSeparator />
          {editor && (
            <DropdownMenuItem onSelect={() => setDialog("import")}>
              <Upload aria-hidden />
              {strings.pages.transfer.importInto}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem disabled={exporter.isPending} onSelect={() => exportAs(false)}>
            <Download aria-hidden />
            {strings.pages.transfer.exportPage}
          </DropdownMenuItem>
          <DropdownMenuItem disabled={exporter.isPending} onSelect={() => exportAs(true)}>
            <Download aria-hidden />
            {strings.pages.transfer.exportTree}
          </DropdownMenuItem>
          {editor && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => setDialog("move")}>
                <FolderInput aria-hidden />
                {copy.moveTo}
              </DropdownMenuItem>
              {onMoveUp && (
                <DropdownMenuItem onSelect={onMoveUp}>
                  <ArrowUp aria-hidden />
                  {copy.moveUp}
                </DropdownMenuItem>
              )}
              {onMoveDown && (
                <DropdownMenuItem onSelect={onMoveDown}>
                  <ArrowDown aria-hidden />
                  {copy.moveDown}
                </DropdownMenuItem>
              )}
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onSelect={() =>
                  trash.mutate(page.id, {
                    onSuccess: () =>
                      nav.setNotice({ tone: "info", text: copy.trashed(title), restore: page.id }),
                    onError: report,
                  })
                }
              >
                <Trash2 aria-hidden />
                {copy.trash}
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      {dialog === "move" && (
        <MoveDialog
          workspace={workspace}
          page={page}
          open
          onOpenChange={(open) => setDialog(open ? "move" : null)}
          onMoved={(moved) =>
            nav.setNotice({ tone: "info", text: strings.pages.tree.dropped(moved) })
          }
        />
      )}
      {dialog === "share" && (
        <ShareDialog
          workspace={workspace}
          page={page}
          open
          onOpenChange={(open) => setDialog(open ? "share" : null)}
        />
      )}
      {dialog === "import" && (
        <ImportDialog
          workspace={workspace}
          place={{ parent_id: page.id }}
          placeName={title}
          open
          onOpenChange={(open) => setDialog(open ? "import" : null)}
          onNavigate={onNavigate}
        />
      )}
    </>
  );
}
