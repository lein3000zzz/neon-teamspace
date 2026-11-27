import { createExtension } from "@blocknote/core";
import { Plugin, PluginKey, TextSelection, type EditorState } from "prosemirror-state";
import { Decoration, DecorationSet, type EditorView } from "prosemirror-view";
import {
  absolutePositionToRelativePosition,
  relativePositionToAbsolutePosition,
  ySyncPluginKey,
} from "y-prosemirror";
import * as Y from "yjs";

/** An inline thread's range, as the server keeps it: two Yjs relative positions in base64. */
export type ThreadAnchor = { id: string; start: string; end: string };

/** A range of the page's text to start a thread on, as the server takes it. */
export type DraftAnchor = { start: string; end: string; quote: string };

/**
 * What the highlights tell the page: a thread to show, from a click or from
 * the keyboard, which moves focus to it; or a range to comment on.
 */
export type CommentHandlers = {
  open: (threadId: string, fromKeyboard: boolean) => void;
  draft: (anchor: DraftAnchor) => void;
};

type State = {
  anchors: readonly ThreadAnchor[];
  active: string | null;
  set: DecorationSet;
};

const key = new PluginKey<State>("neon-comment-highlights");

/** The highlights' extension, by its key, as the editor registers it. */
export const COMMENT_HIGHLIGHTS = "neonCommentHighlights";

function toBase64(bytes: Uint8Array): string {
  let text = "";
  for (const byte of bytes) {
    text += String.fromCharCode(byte);
  }
  return btoa(text);
}

function fromBase64(text: string): Uint8Array {
  return Uint8Array.from(atob(text), (c) => c.charCodeAt(0));
}

/** y-prosemirror's binding of the editor's document to the page's Yjs document. */
type Binding = {
  doc: Y.Doc;
  type: Y.XmlFragment;
  mapping: Map<Y.AbstractType<unknown>, unknown>;
};

function bindingOf(state: EditorState): Binding | undefined {
  const sync = ySyncPluginKey.getState(state) as { binding?: Binding | null } | undefined;
  return sync?.binding ?? undefined;
}

/**
 * Where an anchor is in the editor's document now, from its relative positions;
 * `null` once its text is gone, or before the document holds it.
 */
export function rangeOf(
  state: EditorState,
  anchor: Pick<ThreadAnchor, "start" | "end">,
): { from: number; to: number } | null {
  const binding = bindingOf(state);
  if (binding === undefined) {
    return null;
  }
  try {
    const at = (text: string) =>
      relativePositionToAbsolutePosition(
        binding.doc,
        binding.type,
        Y.decodeRelativePosition(fromBase64(text)),
        binding.mapping as never,
      );
    const from = at(anchor.start);
    const to = at(anchor.end);
    return from === null || to === null || from >= to ? null : { from, to };
  } catch {
    return null;
  }
}

/**
 * The range of text selected in the editor: ProseMirror's selection or, in a
 * read-only editor, which takes no focus and so keeps none, the browser's
 * selection inside it.
 */
function selectedRange(view: EditorView): { from: number; to: number } | null {
  const { from, to, empty } = view.state.selection;
  if (!empty) {
    return { from, to };
  }
  const selected = view.dom.ownerDocument.getSelection();
  if (selected === null || selected.rangeCount === 0 || selected.isCollapsed) {
    return null;
  }
  const range = selected.getRangeAt(0);
  if (!view.dom.contains(range.startContainer) || !view.dom.contains(range.endContainer)) {
    return null;
  }
  try {
    const start = view.posAtDOM(range.startContainer, range.startOffset);
    const end = view.posAtDOM(range.endContainer, range.endOffset);
    return start < end ? { from: start, to: end } : null;
  } catch {
    return null;
  }
}

/** The text selected in the editor as an anchor, with the text it holds; `null` without any. */
export function selectionAnchor(view: EditorView): DraftAnchor | null {
  const range = selectedRange(view);
  const binding = bindingOf(view.state);
  if (range === null || binding === undefined) {
    return null;
  }
  const quote = view.state.doc.textBetween(range.from, range.to, "\n", " ").trim();
  if (quote === "") {
    return null;
  }
  const at = (pos: number) =>
    toBase64(
      Y.encodeRelativePosition(
        absolutePositionToRelativePosition(pos, binding.type, binding.mapping as never),
      ),
    );
  return { start: at(range.from), end: at(range.to), quote };
}

function decorations(state: EditorState, anchors: readonly ThreadAnchor[], active: string | null) {
  const found: Decoration[] = [];
  for (const anchor of anchors) {
    const range = rangeOf(state, anchor);
    if (range !== null) {
      found.push(
        Decoration.inline(
          range.from,
          range.to,
          {
            class: anchor.id === active ? "comment-highlight is-active" : "comment-highlight",
            "data-thread": anchor.id,
          },
          { thread: anchor.id },
        ),
      );
    }
  }
  return DecorationSet.create(state.doc, found);
}

/** The thread of a highlight at `pos`, the innermost first. */
function threadAt(state: EditorState, pos: number): string | undefined {
  const set = key.getState(state)?.set;
  const found = set?.find(pos, pos) ?? [];
  const last = found.at(-1)?.spec as { thread?: string } | undefined;
  return last?.thread;
}

/**
 * Highlights the text each inline thread is on, from the threads' relative
 * positions read against the page's Yjs document whenever they or the
 * document change from elsewhere, and mapped through the user's own edits.
 * A click on a highlight opens its thread; Mod-Alt-M opens the thread under
 * the caret, or starts one on the text selected.
 */
export function commentHighlights(handlers: CommentHandlers) {
  return createExtension({
    key: COMMENT_HIGHLIGHTS,
    prosemirrorPlugins: [
      new Plugin<State>({
        key,
        state: {
          init: () => ({ anchors: [], active: null, set: DecorationSet.empty }),
          apply(tr, value, _old, state) {
            const told = tr.getMeta(key) as Partial<Pick<State, "anchors" | "active">> | undefined;
            const synced = (tr.getMeta(ySyncPluginKey) as { isChangeOrigin?: boolean } | undefined)
              ?.isChangeOrigin;
            if (told === undefined && synced !== true) {
              return { ...value, set: value.set.map(tr.mapping, tr.doc) };
            }
            const anchors = told?.anchors ?? value.anchors;
            const active = told?.active === undefined ? value.active : told.active;
            return { anchors, active, set: decorations(state, anchors, active) };
          },
        },
        props: {
          decorations: (state) => key.getState(state)?.set,
          handleClick(view, pos) {
            const thread = threadAt(view.state, pos);
            if (thread !== undefined) {
              handlers.open(thread, false);
            }
            return false;
          },
          handleKeyDown(view, event) {
            const mod = event.metaKey || event.ctrlKey;
            if (!(mod && event.altKey && event.code === "KeyM")) {
              return false;
            }
            const anchor = selectionAnchor(view);
            if (anchor !== null) {
              handlers.draft(anchor);
              return true;
            }
            const thread = threadAt(view.state, view.state.selection.from);
            if (thread !== undefined) {
              handlers.open(thread, true);
              return true;
            }
            return false;
          },
        },
      }),
    ],
  });
}

/** Tells the highlights which threads to show, and which of them is open. */
export function showThreads(
  view: EditorView,
  anchors: readonly ThreadAnchor[],
  active: string | null,
) {
  view.dispatch(view.state.tr.setMeta(key, { anchors, active }).setMeta("addToHistory", false));
}

/**
 * Scrolls an open thread's text to the middle of the window, leaving the
 * selection and focus where they are; false once its text is gone, or before
 * the document holds it.
 */
export function scrollToThread(view: EditorView, anchor: Pick<ThreadAnchor, "start" | "end">) {
  const range = rangeOf(view.state, anchor);
  if (range === null) {
    return false;
  }
  const { node, offset } = view.domAtPos(range.from, 1);
  const at = node.nodeType === Node.TEXT_NODE ? node : (node.childNodes[offset] ?? node);
  const element = at instanceof Element ? at : at.parentElement;
  element?.scrollIntoView({ block: "center" });
  return true;
}

/** Selects an open thread's text and scrolls it into view; false once it is gone. */
export function revealThread(view: EditorView, anchor: Pick<ThreadAnchor, "start" | "end">) {
  const range = rangeOf(view.state, anchor);
  if (range === null) {
    return false;
  }
  view.dispatch(
    view.state.tr
      .setSelection(TextSelection.create(view.state.doc, range.from, range.to))
      .scrollIntoView(),
  );
  view.focus();
  return true;
}
