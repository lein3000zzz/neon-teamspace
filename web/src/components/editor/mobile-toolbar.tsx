import { useEditorFocus, useEditorState } from "@blocknote/react";
import { cn } from "cn";
import {
  ArrowDown,
  ArrowUp,
  AtSign,
  Bold,
  Captions,
  Check,
  ChevronLeft,
  Code,
  Copy,
  Download,
  Ellipsis,
  FileUp,
  Heading1,
  Heading2,
  Heading3,
  Image,
  IndentDecrease,
  IndentIncrease,
  Italic,
  KeyboardOff,
  Link,
  List,
  ListOrdered,
  ListTodo,
  MessageSquarePlus,
  Minus,
  Paperclip,
  Pilcrow,
  Plus,
  Quote,
  Redo2,
  SquareCode,
  SquareSlash,
  Strikethrough,
  Trash2,
  Type,
  Undo2,
  Unlink,
  type LucideIcon,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ComponentProps,
  type HTMLInputTypeAttribute,
  type ReactNode,
} from "react";
import type { EditorView } from "prosemirror-view";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { openApart } from "@/lib/links";
import { strings } from "@/lib/strings";
import { useEditorPage } from "./editor-page";
import {
  BLOCK_TYPES,
  blockTypeOf,
  captioned,
  currentBlocks,
  duplicate,
  fileBlockOf,
  historyState,
  insertBlock,
  insertUpload,
  linkAddress,
  MARKS,
  move,
  movable,
  onHistoryChange,
  openMenu,
  remove,
  replaceFile,
  selectsText,
  selectTapped,
  setCaption,
  toggleMark,
  turnable,
  turnInto,
  type BlockTypeKey,
  type Editor,
  type Mark,
} from "./toolbar-actions";
import { bottomOf, useVisualViewport } from "./visual-viewport";

const copy = strings.pages.editor.toolbar;

/** What the toolbar shows: its main row, or one of the rows its buttons open in its place. */
type Panel = "main" | "type" | "insert" | "block" | "link" | "caption";

const TYPE_ICONS: Record<BlockTypeKey, LucideIcon> = {
  paragraph: Pilcrow,
  heading1: Heading1,
  heading2: Heading2,
  heading3: Heading3,
  bulletList: List,
  numberedList: ListOrdered,
  checkList: ListTodo,
  quote: Quote,
  code: SquareCode,
};

const MARK_ICONS: Record<Mark, LucideIcon> = {
  bold: Bold,
  italic: Italic,
  strike: Strikethrough,
  code: Code,
};

/** The room kept between the caret and the toolbar above which it must show. */
const CLEARANCE = 12;

/** What the toolbar shows of the editor, read again as the editor changes. */
function snapshot(editor: Editor) {
  const blocks = currentBlocks(editor);
  const styles = editor.getActiveStyles() as Record<string, unknown>;
  const first = blocks[0];
  const file = fileBlockOf(editor);
  return {
    marks: Object.fromEntries(MARKS.map((mark) => [mark, styles[mark] === true])) as Record<
      Mark,
      boolean
    >,
    type: first === undefined ? null : blockTypeOf(first),
    turnable: turnable(blocks),
    nest: editor.canNestBlock(),
    unnest: editor.canUnnestBlock(),
    move: movable(editor, blocks),
    text: selectsText(editor),
    link: editor.getSelectedLinkUrl() ?? null,
    file:
      file === null
        ? null
        : {
            id: file.id,
            url: String((file.props as { url?: unknown }).url ?? ""),
            caption: captioned(editor, file)
              ? String((file.props as { caption?: unknown }).caption ?? "")
              : null,
          },
  };
}

/**
 * Whether there is anything to undo and to redo, as it changes: read apart
 * from the editor's state, which an undo or a redo changes before the
 * history knows there is something to put back.
 */
function useHistory(editor: Editor): { undo: boolean; redo: boolean } {
  const subscribe = useCallback(
    (changed: () => void) => onHistoryChange(editor, changed),
    [editor],
  );
  const undo = useSyncExternalStore(subscribe, () => historyState(editor).undo);
  const redo = useSyncExternalStore(subscribe, () => historyState(editor).redo);
  return { undo, redo };
}

/** The nearest element around the content that scrolls it, or none when the page itself does. */
function scrollerOf(element: Element): Element | null {
  for (let at = element.parentElement; at !== null; at = at.parentElement) {
    const { overflowY } = getComputedStyle(at);
    if ((overflowY === "auto" || overflowY === "scroll") && at.scrollHeight > at.clientHeight) {
      return at;
    }
  }
  return null;
}

/**
 * Scrolls the caret above the toolbar when the toolbar would cover it: the
 * browser and the editor scroll it above the keyboard, which the toolbar sits
 * on, but not clear of the toolbar itself.
 */
function keepClear(view: EditorView, bar: HTMLElement) {
  const box = bar.getBoundingClientRect();
  if (box.height === 0) {
    return;
  }
  let caret: { bottom: number };
  try {
    caret = view.coordsAtPos(view.state.selection.head);
  } catch {
    return;
  }
  const overlap = caret.bottom + CLEARANCE - box.top;
  if (overlap <= 0) {
    return;
  }
  const scroller = scrollerOf(view.dom);
  if (scroller === null) {
    window.scrollBy(0, overlap);
  } else {
    scroller.scrollTop += overlap;
  }
}

type ToolProps = Omit<ComponentProps<typeof Button>, "children"> & {
  icon: LucideIcon;
  /** The name it is read by; shown beside the icon when `shown`. */
  label: string;
  shown?: boolean;
  pressed?: boolean;
};

/** A button of the toolbar: an icon, a full touch target, pressed while what it sets is on. */
function Tool({ icon: Icon, label, shown = false, pressed, className, ...props }: ToolProps) {
  return (
    <Button
      type="button"
      variant="ghost"
      size={shown ? "default" : "icon"}
      aria-label={shown ? undefined : label}
      aria-pressed={pressed}
      className={cn(
        "shrink-0 aria-pressed:bg-accent aria-pressed:text-foreground",
        shown && "px-3",
        className,
      )}
      {...props}
    >
      <Icon aria-hidden />
      {shown && label}
    </Button>
  );
}

/** A row a button of the main row opens: a group of its own, as wide as its buttons. */
const panelClass = "m-0 flex shrink-0 items-center gap-0.5 border-0 p-0";

function Divider() {
  return <span aria-hidden className="mx-0.5 h-6 w-px shrink-0 bg-border" />;
}

/**
 * The editing toolbar of a touch screen, in place of the formatting toolbar
 * that follows a selection made with a mouse. It sits on the bottom of the
 * visual viewport, so above the on-screen keyboard where one covers the page,
 * while the content or the toolbar itself has focus and the user edits the
 * page, and scrolls the caret clear of it. Its buttons format the text
 * selected, or what is typed next, change and move the block the caret is in,
 * insert, comment, undo and redo, and put the keyboard away; taps on it keep
 * the focus, and so the keyboard, where they are. Its rows for a block's type,
 * for inserting, for the block's menu, for a link and for a caption take the
 * main row's place, one at a time.
 */
export function MobileToolbar({ editor }: { editor: Editor }) {
  const page = useEditorPage();
  const focused = useEditorFocus({ includeEditorUI: true }, editor);
  const shown = focused && page.editable;
  const box = useVisualViewport();
  const state = useEditorState({
    editor,
    selector: ({ editor: current }) => snapshot(current as Editor),
  });
  const history = useHistory(editor);
  const [panel, setPanel] = useState<Panel>("main");
  // Hidden, the toolbar starts from its main row when it shows again.
  const [wasShown, setWasShown] = useState(shown);
  if (shown !== wasShown) {
    setWasShown(shown);
    if (!shown) {
      setPanel("main");
    }
  }
  const bar = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(0);
  const imageInput = useRef<HTMLInputElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  // A press anywhere on the bar but its field leaves focus in the content.
  useEffect(() => {
    const element = bar.current;
    if (element === null) {
      return;
    }
    const keep = (event: MouseEvent) => {
      if (!(event.target instanceof HTMLInputElement)) {
        event.preventDefault();
      }
    };
    element.addEventListener("mousedown", keep);
    return () => element.removeEventListener("mousedown", keep);
  }, []);

  // A tap on an image, a file, a divider or a database selects it, for the block menu.
  useEffect(() => {
    const content = editor.domElement;
    if (content === undefined) {
      return;
    }
    const tap = (event: MouseEvent) => {
      selectTapped(editor, event.target);
    };
    content.addEventListener("click", tap);
    return () => content.removeEventListener("click", tap);
  }, [editor]);

  useEffect(() => {
    const element = bar.current;
    if (element === null) {
      return;
    }
    const observer = new ResizeObserver(() => setHeight(element.offsetHeight));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // As the caret moves, and as the keyboard comes or goes, the caret stays above the bar.
  useEffect(() => {
    if (!shown) {
      return;
    }
    let frame = 0;
    const check = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (bar.current !== null) {
          keepClear(editor.prosemirrorView, bar.current);
        }
      });
    };
    check();
    const stop = editor.onSelectionChange(check);
    const viewport = window.visualViewport;
    viewport?.addEventListener("resize", check);
    return () => {
      cancelAnimationFrame(frame);
      stop();
      viewport?.removeEventListener("resize", check);
    };
  }, [editor, shown]);

  const pick = (type: "image" | "file", input: HTMLInputElement) => {
    const file = input.files?.[0];
    input.value = "";
    if (file !== undefined) {
      void insertUpload(editor, type, file);
    }
  };
  /** Puts the keyboard away: from the content, or from the toolbar's own field. */
  const putAway = () => {
    const active = document.activeElement;
    if (active instanceof HTMLElement && bar.current?.contains(active) === true) {
      active.blur();
    }
    editor.blur();
  };
  /** Runs an action of a row, then goes back to the main row. */
  const then = (action: () => void) => () => {
    action();
    setPanel("main");
  };

  const typeLabel = state.type === null ? copy.otherType : copy.types[state.type];
  const TypeIcon = state.type === null ? Type : TYPE_ICONS[state.type];
  const back = <Tool icon={ChevronLeft} label={copy.back} onClick={() => setPanel("main")} />;
  const file = state.file === null ? undefined : editor.getBlock(state.file.id);

  let row: ReactNode;
  switch (panel) {
    case "type":
      row = (
        <fieldset aria-label={copy.blockItems.turnInto} className={panelClass}>
          {back}
          {BLOCK_TYPES.map(({ key }) => (
            <Tool
              key={key}
              icon={TYPE_ICONS[key]}
              label={copy.types[key]}
              shown
              pressed={state.type === key}
              onClick={then(() => turnInto(editor, key))}
            />
          ))}
        </fieldset>
      );
      break;
    case "insert":
      row = (
        <fieldset aria-label={copy.insert} className={panelClass}>
          {back}
          <Tool
            icon={AtSign}
            label={copy.insertItems.mention}
            shown
            onClick={then(() => openMenu(editor, "@"))}
          />
          <Tool
            icon={Image}
            label={copy.insertItems.image}
            shown
            onClick={() => {
              imageInput.current?.click();
              setPanel("main");
            }}
          />
          <Tool
            icon={Paperclip}
            label={copy.insertItems.file}
            shown
            onClick={() => {
              fileInput.current?.click();
              setPanel("main");
            }}
          />
          <Tool
            icon={Minus}
            label={copy.insertItems.divider}
            shown
            onClick={then(() => insertBlock(editor, "divider"))}
          />
          <Tool
            icon={SquareSlash}
            label={copy.insertItems.more}
            shown
            onClick={then(() => openMenu(editor, "/"))}
          />
        </fieldset>
      );
      break;
    case "block":
      row = (
        <fieldset aria-label={copy.block} className={panelClass}>
          {back}
          <Tool
            icon={ArrowUp}
            label={copy.blockItems.moveUp}
            shown
            disabled={!state.move.up}
            onClick={() => move(editor, "up")}
          />
          <Tool
            icon={ArrowDown}
            label={copy.blockItems.moveDown}
            shown
            disabled={!state.move.down}
            onClick={() => move(editor, "down")}
          />
          <Tool
            icon={TypeIcon}
            label={copy.blockItems.turnInto}
            shown
            disabled={!state.turnable}
            onClick={() => setPanel("type")}
          />
          {file !== undefined && (
            <Tool
              icon={FileUp}
              label={copy.blockItems.replaceFile}
              shown
              onClick={then(() => replaceFile(editor, file))}
            />
          )}
          {state.file !== null && state.file.url !== "" && (
            <Tool
              icon={Download}
              label={copy.blockItems.download}
              shown
              onClick={then(() => openApart(state.file?.url ?? ""))}
            />
          )}
          {typeof state.file?.caption === "string" && (
            <Tool
              icon={Captions}
              label={copy.blockItems.caption}
              shown
              onClick={() => setPanel("caption")}
            />
          )}
          <Tool
            icon={Copy}
            label={copy.blockItems.duplicate}
            shown
            onClick={then(() => duplicate(editor))}
          />
          <Tool
            icon={Trash2}
            label={copy.blockItems.delete}
            shown
            onClick={then(() => remove(editor))}
          />
        </fieldset>
      );
      break;
    case "link":
      row = (
        <FieldRow
          editor={editor}
          label={copy.linkAddress}
          placeholder={copy.linkPlaceholder}
          type="url"
          initial={state.link ?? ""}
          apply={{
            label: copy.applyLink,
            run: (address) => {
              if (address.trim() === "") {
                return false;
              }
              if (state.text) {
                editor.createLink(linkAddress(address));
              } else {
                const at = editor.prosemirrorState.selection.anchor;
                const link = editor.getLinkMarkAtPos(at);
                if (link !== undefined) {
                  editor.editLink(linkAddress(address), link.text, at);
                }
              }
              return true;
            },
          }}
          {...(state.link === null
            ? {}
            : { clear: { label: copy.removeLink, run: () => editor.deleteLink() } })}
          onDone={() => setPanel("main")}
        />
      );
      break;
    case "caption":
      row =
        file === undefined ? (
          back
        ) : (
          <FieldRow
            editor={editor}
            label={copy.captionField}
            type="text"
            initial={state.file?.caption ?? ""}
            apply={{
              label: copy.applyCaption,
              run: (caption) => {
                setCaption(editor, file, caption);
                return true;
              },
            }}
            onDone={() => setPanel("main")}
          />
        );
      break;
    case "main":
      row = (
        <>
          <Tool
            icon={TypeIcon}
            label={copy.blockType(typeLabel)}
            disabled={!state.turnable}
            onClick={() => setPanel("type")}
          />
          <Divider />
          {MARKS.map((mark) => (
            <Tool
              key={mark}
              icon={MARK_ICONS[mark]}
              label={copy.marks[mark]}
              pressed={state.marks[mark]}
              onClick={() => toggleMark(editor, mark)}
            />
          ))}
          <Tool
            icon={Link}
            label={copy.link}
            pressed={state.link !== null}
            disabled={!state.text && state.link === null}
            onClick={() => setPanel("link")}
          />
          <Divider />
          <Tool
            icon={IndentIncrease}
            label={copy.indent}
            disabled={!state.nest}
            onClick={() => editor.nestBlock()}
          />
          <Tool
            icon={IndentDecrease}
            label={copy.outdent}
            disabled={!state.unnest}
            onClick={() => editor.unnestBlock()}
          />
          <Divider />
          <Tool icon={Plus} label={copy.insert} onClick={() => setPanel("insert")} />
          <Tool icon={Ellipsis} label={copy.block} onClick={() => setPanel("block")} />
          <Tool
            icon={MessageSquarePlus}
            label={copy.comment}
            disabled={!state.text}
            onClick={page.commentOnSelection}
          />
          <Divider />
          <Tool
            icon={Undo2}
            label={copy.undo}
            disabled={!history.undo}
            onClick={() => editor.undo()}
          />
          <Tool
            icon={Redo2}
            label={copy.redo}
            disabled={!history.redo}
            onClick={() => editor.redo()}
          />
        </>
      );
      break;
  }

  return (
    <>
      {/* Room below the content, so its last line scrolls up above the toolbar and the keyboard. */}
      {shown && <div aria-hidden style={{ height: height + box.covered }} />}
      <div
        ref={bar}
        role="toolbar"
        aria-label={copy.label}
        data-editing-toolbar=""
        hidden={!shown}
        className="fixed top-0 left-0 z-40"
        style={bottomOf(box)}
      >
        <div
          className={cn(
            "flex items-center gap-1 border-t border-border bg-raised pt-1 safe-right-1 safe-left-1",
            box.covered > 0 ? "pb-1" : "safe-bottom-1",
          )}
        >
          {/* Each row afresh, so it starts from its first button however far the last scrolled. */}
          <div
            key={panel}
            className="flex min-w-0 flex-1 touch-pan-x [scrollbar-width:none] items-center gap-0.5 overflow-x-auto overscroll-x-contain"
          >
            {row}
          </div>
          <Divider />
          <Tool icon={KeyboardOff} label={copy.done} onClick={putAway} />
        </div>
      </div>
      <input
        ref={imageInput}
        type="file"
        accept="image/*"
        hidden
        tabIndex={-1}
        aria-label={copy.insertItems.image}
        onChange={(event) => pick("image", event.currentTarget)}
      />
      <input
        ref={fileInput}
        type="file"
        hidden
        tabIndex={-1}
        aria-label={copy.insertItems.file}
        onChange={(event) => pick("file", event.currentTarget)}
      />
    </>
  );
}

type FieldRowProps = {
  editor: Editor;
  /** The field's name, and what it holds when empty. */
  label: string;
  placeholder?: string;
  type: HTMLInputTypeAttribute;
  initial: string;
  /** Sets what was typed; false keeps the row open, as for an empty address. */
  apply: { label: string; run: (value: string) => boolean };
  /** Takes it away, where there is something to take. */
  clear?: { label: string; run: () => void };
  onDone: () => void;
};

/**
 * A field in the toolbar's own row, for what the editor takes as text rather
 * than formatting, such as a link's address or a caption. It takes focus, and
 * so the keyboard, as it opens; focus goes back to the content once what was
 * typed is set or taken away, or Back or Escape leaves it.
 */
function FieldRow({
  editor,
  label,
  placeholder,
  type,
  initial,
  apply,
  clear,
  onDone,
}: FieldRowProps) {
  const [value, setValue] = useState(initial);
  const field = useRef<HTMLInputElement>(null);
  useEffect(() => field.current?.focus(), []);
  const finish = () => {
    editor.focus();
    onDone();
  };
  return (
    <form
      // An address without its scheme is taken, and given one, rather than refused.
      noValidate
      className="flex min-w-0 flex-1 items-center gap-0.5"
      onSubmit={(event) => {
        event.preventDefault();
        if (apply.run(value)) {
          finish();
        }
      }}
    >
      <Tool icon={ChevronLeft} label={copy.back} onClick={finish} />
      <Input
        ref={field}
        type={type}
        {...(type === "url"
          ? { inputMode: "url" as const, autoCapitalize: "none", spellCheck: false }
          : {})}
        enterKeyHint="done"
        autoComplete="off"
        aria-label={label}
        placeholder={placeholder ?? label}
        value={value}
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            finish();
          }
        }}
        className="min-w-32 flex-1"
      />
      <Tool icon={Check} label={apply.label} type="submit" />
      {clear !== undefined && (
        <Tool
          icon={Unlink}
          label={clear.label}
          onClick={() => {
            clear.run();
            finish();
          }}
        />
      )}
    </form>
  );
}
