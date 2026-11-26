import { createContext, use } from "react";
import type { Someone } from "@/lib/people";

/** What the blocks in a page's editor know of the page they are in. */
export type EditorPage = {
  pageId: string;
  /** The workspace's slug, for the links of page mentions. */
  slug: string;
  /** Everyone mentions may name, by id; empty for a guest, who knows nobody. */
  people: ReadonlyMap<string, Someone>;
  /** Whether the user edits the page, and so its blocks. */
  editable: boolean;
  /** Whether databases can be made in the page: not in a row, nor a database. */
  holdsDatabases: boolean;
  /** Database blocks whose database is being made, by block id. */
  creating: ReadonlySet<string>;
  /** Makes an inline database in the page and gives it to the block. */
  createIn: (blockId: string) => void;
  /** Starts a thread of comments on the text selected, from the formatting toolbar. */
  commentOnSelection: () => void;
};

export const EditorPageContext = createContext<EditorPage | null>(null);

/** The page the editor shows, inside its blocks. */
export function useEditorPage(): EditorPage {
  const page = use(EditorPageContext);
  if (page === null) {
    throw new Error("useEditorPage must be used inside a page's editor");
  }
  return page;
}
