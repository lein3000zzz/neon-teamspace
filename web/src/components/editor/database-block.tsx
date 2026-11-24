import { createReactBlockSpec } from "@blocknote/react";
import { Database } from "lucide-react";
import { strings } from "@/lib/strings";
import { databaseBlockConfig } from "./custom-blocks";

const copy = strings.pages.editor.database;

/**
 * The database block as the editor draws it: a placeholder that names the
 * database the block shows, by its id.
 */
export const databaseBlock = createReactBlockSpec(databaseBlockConfig, {
  render: ({ block }) => <DatabasePlaceholder databaseId={block.props.databaseId} />,
});

function DatabasePlaceholder({ databaseId }: { databaseId: string }) {
  return (
    <div className="flex w-full min-w-0 items-center gap-2 rounded-md border border-border px-3 py-2">
      <Database aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      <span>{copy.label}</span>
      <span className="min-w-0 truncate font-mono text-xs text-muted-foreground">
        {databaseId === "" ? copy.none : databaseId}
      </span>
    </div>
  );
}
