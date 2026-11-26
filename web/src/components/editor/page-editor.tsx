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
  FormattingToolbar,
  FormattingToolbarController,
  getDefaultReactSlashMenuItems,
  getFormattingToolbarItems,
  SuggestionMenuController,
  useComponentsContext,
  useCreateBlockNote,
  type DefaultReactSuggestionItem,
  type FormattingToolbarProps,
} from "@blocknote/react";
import { BlockNoteView } from "@blocknote/shadcn";
import { useQueryClient } from "@tanstack/react-query";
import { Database, FileText, MessageSquarePlus, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode, type RefObject } from "react";
import { mentionsQuery } from "@/api/comments";
import { useCreateDatabase } from "@/api/databases";
import type { Person } from "@/api/workspaces";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { HueDot } from "@/components/nodes/hue-dot";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { titleOf } from "@/lib/page-tree";
import { attribution, type Someone } from "@/lib/people";
import { strings } from "@/lib/strings";
import { useTheme } from "@/theme/theme-provider";
import {
  COMMENT_HIGHLIGHTS,
  commentHighlights,
  revealThread,
  selectionAnchor,
  showThreads,
  type CommentHandlers,
  type DraftAnchor,
  type ThreadAnchor,
} from "./comment-highlights";
import { FRAGMENT, useSyncSession, type SyncSession } from "./connection";
import type { Connection } from "./connection-state";
import { collaborator, renderCursor, type Collaborator } from "./cursor";
import { EditorPageContext, useEditorPage, type EditorPage } from "./editor-page";
import { pasteFits } from "./paste";
import { schema } from "./schema";

const copy = strings.pages.editor;

/** What the page asks of its editor, for its comments and the links that open it. */
export type EditorHandle = {
  /** The text selected, as an anchor to start a thread on; `null` without any. */
  selection: () => DraftAnchor | null;
  /** Selects a thread's text and scrolls to it; false once its text is gone. */
  reveal: (anchor: Pick<ThreadAnchor, "start" | "end">) => boolean;
  /** Scrolls to a block and puts the caret at its start; false when the page holds none such. */
  showBlock: (blockId: string) => boolean;
};

/** The page's comments, as its editor shows them and tells of them. */
export type EditorComments = {
  /** The open inline threads, each highlighted on its text. */
  threads: readonly ThreadAnchor[];
  /** The thread open in the comments panel, its highlight marked. */
  active: string | null;
  /**
   * A highlight was clicked, or Mod-Alt-M pressed on one; or a range chosen to
   * comment on. New ones register the highlights afresh, so they should keep their
   * identity; `changed` likewise, as a new one opens the connection afresh.
   */
  handlers: CommentHandlers;
  /** A thread of the page changed, as its connection tells. */
  changed: () => void;
};

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
  /** Filled with what the page asks of the editor for its comments. */
  handleRef: RefObject<EditorHandle | null>;
  comments: EditorComments;
  /** Everyone mentions may name, by id; empty for a guest. */
  people: ReadonlyMap<string, Someone>;
  /** Someone was mentioned who may not see the page, and so is told nothing. */
  onUnseen: (person: Person) => void;
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
  handleRef,
  comments,
  people,
  onUnseen,
}: PageEditorProps) {
  const { changed } = comments;
  const { session, connection } = useSyncSession(pageId, user.id, onAccessChanged, changed);

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
      handleRef={handleRef}
      comments={comments}
      people={people}
      onUnseen={onUnseen}
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
  handleRef: RefObject<EditorHandle | null>;
  comments: EditorComments;
  people: ReadonlyMap<string, Someone>;
  onUnseen: (person: Person) => void;
};

function LiveEditor({
  pageId,
  holdsDatabases,
  session,
  editable,
  self,
  focusRef,
  handleRef,
  comments,
  people,
  onUnseen,
}: LiveEditorProps) {
  const { mode } = useTheme();
  const workspace = useWorkspace();
  const queryClient = useQueryClient();
  const createDatabase = useCreateDatabase(workspace.id);
  const [refused, setRefused] = useState(false);
  const [creating, setCreating] = useState<ReadonlySet<string>>(() => new Set());
  const [notMade, setNotMade] = useState<unknown>(null);
  const { handlers } = comments;
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

  // Registered once the editor is made: given among its options, an extension
  // keeps the hook from inferring the editor's schema.
  useEffect(() => {
    editor.registerExtension(commentHighlights(handlers));
    return () => {
      editor.unregisterExtension(COMMENT_HIGHLIGHTS);
    };
  }, [editor, handlers]);

  useEffect(() => {
    focusRef.current = () => editor.focus();
    return () => {
      focusRef.current = null;
    };
  }, [editor, focusRef]);

  useEffect(() => {
    handleRef.current = {
      selection: () => selectionAnchor(editor.prosemirrorView),
      reveal: (anchor) => revealThread(editor.prosemirrorView, anchor),
      showBlock: (blockId) => {
        if (editor.getBlock(blockId) === undefined) {
          return false;
        }
        editor.setTextCursorPosition(blockId, "start");
        const blocks = editor.domElement?.querySelectorAll("[data-node-type='blockOuter']") ?? [];
        [...blocks]
          .find((block) => block.getAttribute("data-id") === blockId)
          ?.scrollIntoView({ block: "center" });
        editor.focus();
        return true;
      },
    };
    return () => {
      handleRef.current = null;
    };
  }, [editor, handleRef]);

  const { threads, active } = comments;
  useEffect(() => {
    showThreads(editor.prosemirrorView, threads, active);
  }, [editor, threads, active]);

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
      slug: workspace.slug,
      people,
      editable,
      holdsDatabases,
      creating,
      createIn: (blockId: string) => void createIn(blockId),
      commentOnSelection: () => {
        const anchor = selectionAnchor(editor.prosemirrorView);
        if (anchor !== null) {
          handlers.draft(anchor);
        }
      },
    }),
    [
      pageId,
      workspace.slug,
      people,
      editable,
      holdsDatabases,
      creating,
      createIn,
      editor,
      handlers,
    ],
  );
  /** The people and pages `@` may mention, as the server finds them for what follows it. */
  const mentionItems = useCallback(
    async (query: string): Promise<DefaultReactSuggestionItem[]> => {
      const found = await queryClient.fetchQuery(mentionsQuery(workspace.id, pageId, query));
      const mentionCopy = strings.comments;
      return [
        ...found.people.map(({ person, sees_page }) => ({
          title: attribution(person),
          ...(sees_page ? {} : { subtext: mentionCopy.cannotSee }),
          icon: <HueDot hue={person.hue} className="size-2" />,
          onItemClick: () => {
            editor.insertInlineContent([
              {
                type: "mention",
                props: { kind: "user", target: person.id, label: person.display_name },
              },
              " ",
            ]);
            if (!sees_page) {
              onUnseen(person);
            }
          },
        })),
        ...found.pages.map((linked) => ({
          title: titleOf(linked),
          subtext: mentionCopy.page,
          icon:
            linked.icon === undefined || linked.icon === null ? (
              <FileText aria-hidden size={18} />
            ) : (
              <span aria-hidden>{linked.icon}</span>
            ),
          onItemClick: () => {
            editor.insertInlineContent([
              {
                type: "mention",
                props: { kind: "page", target: linked.id, label: titleOf(linked) },
              },
              " ",
            ]);
          },
        })),
      ];
    },
    [editor, queryClient, workspace.id, pageId, onUnseen],
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
          formattingToolbar={false}
        >
          <SuggestionMenuController triggerCharacter="/" getItems={slashItems} />
          <SuggestionMenuController triggerCharacter="@" getItems={mentionItems} />
          <FormattingToolbarController formattingToolbar={ToolbarWithComment} />
        </BlockNoteView>
      </EditorPageContext>
    </section>
  );
}

/** BlockNote's formatting toolbar, and last the button to start a thread on the text selected. */
function ToolbarWithComment(props: FormattingToolbarProps) {
  const Components = useComponentsContext();
  const page = useEditorPage();
  return (
    <FormattingToolbar {...props}>
      {...getFormattingToolbarItems(props.blockTypeSelectItems)}
      {Components !== undefined && (
        <Components.FormattingToolbar.Button
          label={strings.comments.toolbar}
          mainTooltip={strings.comments.toolbar}
          icon={<MessageSquarePlus aria-hidden size={18} />}
          onClick={page.commentOnSelection}
        />
      )}
    </FormattingToolbar>
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
