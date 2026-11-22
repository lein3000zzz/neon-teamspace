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
import { X } from "lucide-react";
import { useEffect, useState, type RefObject } from "react";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";
import { useTheme } from "@/theme/theme-provider";
import { FRAGMENT, useSyncSession, type SyncSession } from "./connection";
import type { Connection } from "./connection-state";
import { collaborator, renderCursor, type Collaborator } from "./cursor";
import { pasteFits } from "./paste";

const copy = strings.pages.editor;

export type PageEditorProps = {
  pageId: string;
  /** False while the user only views the page, or it is in the trash. */
  editable: boolean;
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
  session: SyncSession;
  editable: boolean;
  self: Collaborator;
  focusRef: RefObject<(() => void) | null>;
};

function LiveEditor({ session, editable, self, focusRef }: LiveEditorProps) {
  const { mode } = useTheme();
  const [refused, setRefused] = useState(false);
  const editor = useCreateBlockNote(
    {
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
