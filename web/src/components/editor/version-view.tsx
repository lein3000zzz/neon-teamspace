import "@blocknote/shadcn/style.css";
import "@/styles/editor.css";
import type {
  BlockNoteEditor,
  BlockSchema,
  InlineContentSchema,
  StyleSchema,
} from "@blocknote/core";
import { useCreateBlockNote } from "@blocknote/react";
import { BlockNoteView } from "@blocknote/shadcn";
import { useTheme } from "@/theme/theme-provider";
import { EditorPageContext, type EditorPage } from "./editor-page";
import { schema } from "./schema";

/** What a version's blocks know of the page: nothing to edit or make. */
const READ_ONLY: EditorPage = {
  pageId: "",
  editable: false,
  holdsDatabases: false,
  creating: new Set(),
  createIn: () => {},
};

type VersionViewProps = {
  /** The version's blocks, at least one, in BlockNote's JSON, as the server sends them. */
  blocks: unknown[];
};

/**
 * A version of a page as the editor would show it, read-only: an editor of
 * the page's schema holding the version's blocks and no live document, so
 * nothing in it reaches the page. A database block shows its database as it
 * is now, as the version only names it.
 */
export default function VersionView({ blocks }: VersionViewProps) {
  const { mode } = useTheme();
  // The server writes the blocks in the schema's shape, which its fixtures check. The
  // schema's own block type cannot be named under exactOptionalPropertyTypes, as the page
  // editor notes, so they go in untyped.
  const editor = useCreateBlockNote({ schema, initialContent: blocks as never }, [blocks]);
  return (
    <EditorPageContext value={READ_ONLY}>
      <BlockNoteView
        editor={editor as unknown as BlockNoteEditor<BlockSchema, InlineContentSchema, StyleSchema>}
        editable={false}
        theme={mode}
        sideMenu={false}
        slashMenu={false}
        formattingToolbar={false}
        linkToolbar={false}
        filePanel={false}
        tableHandles={false}
      />
    </EditorPageContext>
  );
}
