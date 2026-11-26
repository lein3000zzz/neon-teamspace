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
import { useMemo } from "react";
import type { Someone } from "@/lib/people";
import { useTheme } from "@/theme/theme-provider";
import { EditorPageContext, type EditorPage } from "./editor-page";
import { schema } from "./schema";

type VersionViewProps = {
  /** The version's blocks, at least one, in BlockNote's JSON, as the server sends them. */
  blocks: unknown[];
  /** The workspace's slug, for the links of page mentions. */
  slug: string;
  /** Everyone mentions may name, by id, as the page's editor knows them. */
  people: ReadonlyMap<string, Someone>;
};

/**
 * A version of a page as the editor would show it, read-only: an editor of
 * the page's schema holding the version's blocks and no live document, so
 * nothing in it reaches the page. A database block shows its database as it
 * is now, as the version only names it; a mention shows whom it names as the
 * page's editor would.
 */
export default function VersionView({ blocks, slug, people }: VersionViewProps) {
  const { mode } = useTheme();
  // What a version's blocks know of the page: nothing to edit, make or comment on.
  const page = useMemo<EditorPage>(
    () => ({
      pageId: "",
      slug,
      people,
      editable: false,
      holdsDatabases: false,
      creating: new Set(),
      createIn: () => {},
      commentOnSelection: () => {},
    }),
    [slug, people],
  );
  // The server writes the blocks in the schema's shape, which its fixtures check. The
  // schema's own block type cannot be named under exactOptionalPropertyTypes, as the page
  // editor notes, so they go in untyped.
  const editor = useCreateBlockNote({ schema, initialContent: blocks as never }, [blocks]);
  return (
    <EditorPageContext value={page}>
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
