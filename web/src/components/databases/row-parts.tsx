import { useQueryClient } from "@tanstack/react-query";
import { cn } from "cn";
import { Maximize2, MoreHorizontal, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { dropRow, refreshRows, useChangeRow, type Row } from "@/api/databases";
import { useTrashPage } from "@/api/pages";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { titleProperty } from "@/lib/database-rules";
import { edits } from "@/lib/page-rules";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { useDatabaseScope, useOpenRow } from "./database-scope";

const copy = strings.databases.rows;

type TitleFieldProps = {
  row: Row;
  /** Told once it ends, saved or not, and whether Enter or Escape ended it rather than leaving it. */
  onDone: (fromKeyboard: boolean) => void;
  className?: string;
};

/**
 * A row's title in place, as a new row opens or its title is clicked: Enter
 * or leaving it saves what was typed, Escape keeps the title as it is. A
 * title changed elsewhere meanwhile stays unless something was typed.
 */
export function TitleField({ row, onDone, className }: TitleFieldProps) {
  const workspace = useWorkspace();
  const { database, report } = useDatabaseScope();
  const change = useChangeRow(workspace.id, database, row.id);
  const [title, setTitle] = useState(row.title);
  const [opened] = useState(row.title);
  const field = useRef<HTMLInputElement>(null);
  const finished = useRef(false);
  const titleId = titleProperty(database)?.id;

  useEffect(() => field.current?.focus(), []);

  const finish = (save: boolean, fromKeyboard: boolean) => {
    if (finished.current) {
      return;
    }
    finished.current = true;
    const next = title.replaceAll(/[\r\n]+/g, " ");
    if (save && titleId !== undefined && next !== opened) {
      change.mutate({ [titleId]: next }, { onError: report });
    }
    onDone(fromKeyboard);
  };

  return (
    <input
      ref={field}
      aria-label={copy.title(titleOf(row))}
      placeholder={strings.pages.untitled}
      maxLength={500}
      value={title}
      onChange={(event) => setTitle(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter" && !event.nativeEvent.isComposing) {
          event.preventDefault();
          finish(true, true);
        } else if (event.key === "Escape") {
          event.preventDefault();
          finish(false, true);
        }
      }}
      onBlur={() => finish(true, false)}
      className={cn(
        "h-(--tap-sm) w-full min-w-0 rounded-sm border border-primary bg-background px-2 text-base font-medium md:text-sm",
        className,
      )}
    />
  );
}

/**
 * Puts the focus back on what a field in place of it came from, once the
 * field ends from the keyboard, as Enter and Escape end it; a click elsewhere
 * leaves the focus where it went. `target` goes on that element, and
 * `ended` is told how the field ended.
 */
export function useRefocus<T extends HTMLElement>(editing: boolean) {
  const target = useRef<T>(null);
  const refocus = useRef(false);
  useEffect(() => {
    if (!editing && refocus.current) {
      refocus.current = false;
      target.current?.focus();
    }
  }, [editing]);
  const ended = useCallback((fromKeyboard: boolean) => {
    refocus.current = fromKeyboard;
  }, []);
  return { target, ended };
}

/** A row's icon, or nothing: rows without one show their title alone. */
export function RowIcon({ row }: { row: Row }) {
  const icon = row.icon?.trim() ?? "";
  return icon === "" ? null : (
    <span aria-hidden className="shrink-0 leading-none">
      {icon}
    </span>
  );
}

type RowMenuProps = {
  row: Row;
  /** Further items before the row's own, such as a card's moves. */
  children?: ReactNode;
  className?: string;
};

/** What to do with a row from one button: open it as a full page, or move it to the trash. */
export function RowMenu({ row, children, className }: RowMenuProps) {
  const workspace = useWorkspace();
  const queryClient = useQueryClient();
  const { database, report } = useDatabaseScope();
  const openRow = useOpenRow();
  const trash = useTrashPage(workspace.id);
  const title = titleOf(row);
  const trashed = (database.page.trashed_at ?? null) !== null;
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={copy.more(title)}
          data-no-drag=""
          className={cn("text-muted-foreground", className)}
        >
          <MoreHorizontal aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {children}
        {children !== undefined && <DropdownMenuSeparator />}
        <DropdownMenuItem onSelect={() => openRow(row.id, true)}>
          <Maximize2 aria-hidden />
          {copy.openFull}
        </DropdownMenuItem>
        {edits(row.access) && !trashed && (
          <DropdownMenuItem
            onSelect={() =>
              trash.mutate(row.id, {
                onSuccess: () => {
                  dropRow(queryClient, workspace.id, database.page.id, row.id);
                  refreshRows(queryClient, workspace.id, database.page.id);
                },
                onError: report,
              })
            }
          >
            <Trash2 aria-hidden />
            {copy.trash}
          </DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
