import { createContext, use } from "react";

/** What the blocks in a page's editor know of the page they are in. */
export type EditorPage = {
  pageId: string;
  /** Whether the user edits the page, and so its blocks. */
  editable: boolean;
  /** Whether databases can be made in the page: not in a row, nor a database. */
  holdsDatabases: boolean;
  /** Database blocks whose database is being made, by block id. */
  creating: ReadonlySet<string>;
  /** Makes an inline database in the page and gives it to the block. */
  createIn: (blockId: string) => void;
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
