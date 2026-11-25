import "@blocknote/shadcn/style.css";
import "@/styles/editor.css";
import {
  filterSuggestionItems,
  insertOrUpdateBlockForSlashMenu,
  type BlockNoteEditor,
  type BlockSchema,
  type InlineContentSchema,
  type StyleSchema,
} from "@blocknote/core";
import { withCollaboration } from "@blocknote/core/yjs";
import {
  getDefaultReactSlashMenuItems,
  SuggestionMenuController,
  useCreateBlockNote,
  type DefaultReactSuggestionItem,
} from "@blocknote/react";
import { BlockNoteView } from "@blocknote/shadcn";
import { Database, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode, type RefObject } from "react";
import { useCreateDatabase } from "@/api/databases";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { strings } from "@/lib/strings";
import { useTheme } from "@/theme/theme-provider";
import { FRAGMENT, useSyncSession, type SyncSession } from "./connection";
import type { Connection } from "./connection-state";
import { collaborator, renderCursor, type Collaborator } from "./cursor";
import { EditorPageContext, type EditorPage } from "./editor-page";
import { pasteFits } from "./paste";
import { schema } from "./schema";

const copy = strings.pages.editor;

export type PageEditorProps = {
  pageId: string;
  /** False while the user only views the page, or it is in the trash. */
  editable: boolean;
  /** Whether databases can be made inside the page, which a row or a database cannot hold. */
  holdsDatabases: boolean;
  /** The signed-in user, as the others see them. */
  user: { id: string; name: string; hue: number };
  /** Told how the live connection stands as it changes. */
  onConnection: (connection: Connection) => void;
  /** Runs when the server says what the user may now do, or ends the connection for good. */
  onAccessChanged: () => void;
  /** Filled with a way to put the cursor in the content, for Enter in the title. */
  focusRef: RefObject<(() => void) | null>;
};

/**
 * A page's content: BlockNote, its shadcn variant, bound to the page's Yjs
 * document, which the server keeps and shares with everyone who has the page
 * open. It takes input only while what is typed can reach the server: not
 * before the document came from it, so the first keystrokes never land in an
 * empty copy, and not once the server ended the connection for good, as
 * nothing typed then would be sent. A viewer's editor never takes input, and
 * one the server turns read-only stops at once; the server ignores changes
 * from viewers in any case.
 */
export default function PageEditor({
  pageId,
  editable,
  holdsDatabases,
  user,
  onConnection,
  onAccessChanged,
  focusRef,
}: PageEditorProps) {
  const { session, connection } = useSyncSession(pageId, user.id, onAccessChanged);

  useEffect(() => onConnection(connection), [connection, onConnection]);

  if (session === null) {
    return null;
  }
  return (
    <LiveEditor
      key={session.doc.guid}
      pageId={pageId}
      holdsDatabases={holdsDatabases}
      session={session}
      editable={
        editable &&
        connection.loaded &&
        connection.status !== "closed" &&
        connection.access !== "view"
      }
      self={collaborator(user.id, user.name, user.hue)}
      focusRef={focusRef}
    />
  );
}

type LiveEditorProps = {
  pageId: string;
  holdsDatabases: boolean;
  session: SyncSession;
  editable: boolean;
  self: Collaborator;
  focusRef: RefObject<(() => void) | null>;
};

function LiveEditor({
  pageId,
  holdsDatabases,
  session,
  editable,
  self,
  focusRef,
}: LiveEditorProps) {
  const { mode } = useTheme();
  const workspace = useWorkspace();
  const createDatabase = useCreateDatabase(workspace.id);
  const [refused, setRefused] = useState(false);
  const [creating, setCreating] = useState<ReadonlySet<string>>(() => new Set());
  const [notMade, setNotMade] = useState<unknown>(null);
  const editor = useCreateBlockNote(
    {
      schema,
      ...withCollaboration({
        collaboration: {
          fragment: session.doc.getXmlFragment(FRAGMENT),
          user: { ...self },
          provider: session.provider,
          renderCursor,
          showCursorLabels: "activity",
        },
      }),
      // The server refuses a message over its limit, and the paste would be lost.
      pasteHandler: ({ event, defaultPasteHandler }) => {
        if (event.clipboardData !== null && !pasteFits(event.clipboardData)) {
          setRefused(true);
          return true;
        }
        setRefused(false);
        return defaultPasteHandler();
      },
    },
    [session],
  );

  useEffect(() => {
    focusRef.current = () => editor.focus();
    return () => {
      focusRef.current = null;
    };
  }, [editor, focusRef]);

  const { mutateAsync: makeDatabase } = createDatabase;
  /** Makes an inline database under the page, and has the block show it once it is made. */
  const createIn = useCallback(
    async (blockId: string) => {
      setNotMade(null);
      setCreating((current) => new Set([...current, blockId]));
      try {
        const database = await makeDatabase({ parent_id: pageId, inline: true });
        if (editor.getBlock(blockId) !== undefined) {
          editor.updateBlock(blockId, { props: { databaseId: database.page.id } });
        }
      } catch (error) {
        setNotMade(error);
      } finally {
        setCreating((current) => new Set([...current].filter((id) => id !== blockId)));
      }
    },
    [editor, makeDatabase, pageId],
  );
  const page: EditorPage = useMemo(
    () => ({
      pageId,
      editable,
      holdsDatabases,
      creating,
      createIn: (blockId: string) => void createIn(blockId),
    }),
    [pageId, editable, holdsDatabases, creating, createIn],
  );
  const slashItems = useCallback(
    async (query: string) => {
      const items = getDefaultReactSlashMenuItems(asSchemaless(editor));
      if (!holdsDatabases) {
        return filterSuggestionItems(items, query);
      }
      return filterSuggestionItems(
        withDatabase(items, {
          title: copy.slashDatabase.title,
          subtext: copy.slashDatabase.subtext,
          aliases: ["database", "table", "board", "kanban", "list", "db"],
          icon: <Database aria-hidden size={18} />,
          onItemClick: () => {
            const block = insertOrUpdateBlockForSlashMenu(editor, { type: "database" });
            void createIn(block.id);
          },
        }),
        query,
      );
    },
    [editor, holdsDatabases, createIn],
  );

  return (
    <section aria-label={copy.label} className="grid min-h-48 gap-3">
      {refused && (
        <div style={{ paddingInline: "var(--page-inset)" }}>
          <Alert tone="error" live="assertive">
            <div className="flex items-start gap-2">
              <p className="min-w-0 flex-1 text-foreground">{copy.pasteTooLarge}</p>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={copy.dismiss}
                onClick={() => setRefused(false)}
                className="-my-1"
              >
                <X aria-hidden />
              </Button>
            </div>
          </Alert>
        </div>
      )}
      {notMade !== null && (
        <div style={{ paddingInline: "var(--page-inset)" }}>
          <ErrorNotice error={notMade} title={copy.databaseFailed} />
        </div>
      )}
      <EditorPageContext value={page}>
        <BlockNoteView
          editor={asSchemaless(editor)}
          editable={editable}
          theme={mode}
          slashMenu={false}
        >
          <SuggestionMenuController triggerCharacter="/" getItems={slashItems} />
        </BlockNoteView>
      </EditorPageContext>
    </section>
  );
}

/**
 * The slash menu's items with the database's beside BlockNote's table, in its
 * group, or last when the table is not offered.
 */
function withDatabase(
  items: DefaultReactSuggestionItem[],
  database: Omit<DefaultReactSuggestionItem, "group"> & { icon: ReactNode },
): DefaultReactSuggestionItem[] {
  const table = items.findIndex((item) => item.aliases?.includes("table"));
  const group = items[table]?.group;
  const item = {
    ...database,
    ...(group === undefined ? {} : { group }),
  } as DefaultReactSuggestionItem;
  return table < 0
    ? [...items, item]
    : [...items.slice(0, table + 1), item, ...items.slice(table + 1)];
}

/**
 * The editor as the view's props take it. BlockNote's default schema has
 * optional props, such as a heading's isToggleable, which do not fit its own
 * PropSchema index signature under exactOptionalPropertyTypes, so the view
 * cannot infer it; the editor is the same object either way.
 */
function asSchemaless(
  editor: ReturnType<typeof useCreateBlockNote>,
): BlockNoteEditor<BlockSchema, InlineContentSchema, StyleSchema> {
  return editor as unknown as BlockNoteEditor<BlockSchema, InlineContentSchema, StyleSchema>;
}
