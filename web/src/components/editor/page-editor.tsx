import "@blocknote/shadcn/style.css";
import "@/styles/editor.css";
import type {
  BlockNoteEditor,
  BlockSchema,
  InlineContentSchema,
  StyleSchema,
} from "@blocknote/core";
import { withCollaboration } from "@blocknote/core/yjs";
import { useCreateBlockNote } from "@blocknote/react";
import { BlockNoteView } from "@blocknote/shadcn";
import { useEffect, type RefObject } from "react";
import { strings } from "@/lib/strings";
import { useTheme } from "@/theme/theme-provider";
import { FRAGMENT, useSyncSession, type SyncSession } from "./connection";
import type { Connection } from "./connection-state";
import { collaborator, renderCursor, type Collaborator } from "./cursor";

export type PageEditorProps = {
  pageId: string;
  /** False while the user only views the page, or it is in the trash. */
  editable: boolean;
  /** The signed-in user, as the others see them. */
  user: { id: string; name: string; hue: number };
  /** Told how the live connection stands as it changes. */
  onConnection: (connection: Connection) => void;
  /** Runs when the server ends the connection for good. */
  onClosed: () => void;
  /** Filled with a way to put the cursor in the content, for Enter in the title. */
  focusRef: RefObject<(() => void) | null>;
};

/**
 * A page's content: BlockNote, its shadcn variant, bound to the page's Yjs
 * document, which the server keeps and shares with everyone who has the page
 * open. Nothing can be typed until the document came from the server, so the
 * first keystrokes never land in an empty copy of it. A viewer's editor never
 * takes input; the server ignores changes from them in any case.
 */
export default function PageEditor({
  pageId,
  editable,
  user,
  onConnection,
  onClosed,
  focusRef,
}: PageEditorProps) {
  const { session, connection } = useSyncSession(pageId, user.id, onClosed);

  useEffect(() => onConnection(connection), [connection, onConnection]);

  if (session === null) {
    return null;
  }
  return (
    <LiveEditor
      key={session.doc.guid}
      session={session}
      editable={editable && connection.loaded}
      self={collaborator(user.id, user.name, user.hue)}
      focusRef={focusRef}
    />
  );
}

type LiveEditorProps = {
  session: SyncSession;
  editable: boolean;
  self: Collaborator;
  focusRef: RefObject<(() => void) | null>;
};

function LiveEditor({ session, editable, self, focusRef }: LiveEditorProps) {
  const { mode } = useTheme();
  const editor = useCreateBlockNote(
    withCollaboration({
      collaboration: {
        fragment: session.doc.getXmlFragment(FRAGMENT),
        user: { ...self },
        provider: session.provider,
        renderCursor,
        showCursorLabels: "activity",
      },
    }),
    [session],
  );

  useEffect(() => {
    focusRef.current = () => editor.focus();
    return () => {
      focusRef.current = null;
    };
  }, [editor, focusRef]);

  return (
    <section aria-label={strings.pages.editor.label} className="min-h-48">
      <BlockNoteView editor={asSchemaless(editor)} editable={editable} theme={mode} />
    </section>
  );
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
