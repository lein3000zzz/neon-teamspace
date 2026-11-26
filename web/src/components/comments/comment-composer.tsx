import { useQuery } from "@tanstack/react-query";
import { useId, useRef, useState, type KeyboardEvent } from "react";
import { flushSync } from "react-dom";
import { mentionMarkdown, mentionsQuery } from "@/api/comments";
import type { Person } from "@/api/workspaces";
import { PersonLabel } from "@/components/people/person-label";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";

const copy = strings.comments;

/** What `@` and the text typed after it name, up to the caret. */
const MENTION = /(^|\s)@([^\s@[\]()]{0,40})$/;

type Choice =
  | { kind: "user"; id: string; label: string; person: Person; sees: boolean }
  | { kind: "page"; id: string; label: string };

type CommentComposerProps = {
  workspaceId: string;
  pageId: string;
  /** The field's accessible name. */
  label: string;
  submitLabel: string;
  /** What it holds at first, to change a comment. */
  initial?: string;
  /** Saves the markdown; the field empties once it resolves. */
  onSubmit: (markdown: string) => Promise<unknown>;
  onCancel?: () => void;
  /** Someone mentioned who may not see the page, and will not be told. */
  onUnseen?: (person: Person) => void;
  disabled?: boolean;
};

/**
 * A field for a comment's markdown. Typing `@` offers the people, agents and
 * pages a mention may name, as a list the arrow keys move through and Enter
 * picks; a pick writes the mention's markdown in. Mod-Enter sends.
 */
export function CommentComposer({
  workspaceId,
  pageId,
  label,
  submitLabel,
  initial = "",
  onSubmit,
  onCancel,
  onUnseen,
  disabled = false,
}: CommentComposerProps) {
  const [text, setText] = useState(initial);
  const [caret, setCaret] = useState(initial.length);
  const [highlighted, setHighlighted] = useState(0);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  const listId = useId();
  const typed = MENTION.exec(text.slice(0, caret));
  const query = typed?.[2] ?? null;
  const open = query !== null && dismissed !== text;
  const found = useQuery({
    ...mentionsQuery(workspaceId, pageId, query ?? ""),
    enabled: open,
  });
  const choices: Choice[] = open
    ? [
        ...(found.data?.people ?? []).map((candidate): Choice => ({
          kind: "user",
          id: candidate.person.id,
          label: candidate.person.display_name,
          person: candidate.person,
          sees: candidate.sees_page,
        })),
        ...(found.data?.pages ?? []).map((page): Choice => ({
          kind: "page",
          id: page.id,
          label: titleOf(page),
        })),
      ]
    : [];
  const current = Math.min(highlighted, Math.max(choices.length - 1, 0));

  const pick = (choice: Choice) => {
    const start = caret - (query?.length ?? 0) - 1;
    const written = `${mentionMarkdown(choice.kind, choice.id, choice.label)} `;
    const next = text.slice(0, start) + written + text.slice(caret);
    const at = start + written.length;
    // The caret goes after the mention before anything else is typed.
    flushSync(() => {
      setText(next);
      setCaret(at);
      setHighlighted(0);
    });
    field.current?.setSelectionRange(at, at);
    if (choice.kind === "user" && !choice.sees) {
      onUnseen?.(choice.person);
    }
  };

  const send = async () => {
    if (text.trim() === "" || sending) {
      return;
    }
    setSending(true);
    try {
      await onSubmit(text);
      setText("");
      setCaret(0);
    } catch {
      // The caller shows what went wrong; the text stays to send again.
    } finally {
      setSending(false);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (open && choices.length > 0) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const step = event.key === "ArrowDown" ? 1 : choices.length - 1;
        setHighlighted((current + step) % choices.length);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        const choice = choices[current];
        if (choice !== undefined) {
          event.preventDefault();
          pick(choice);
        }
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setDismissed(text);
        return;
      }
    }
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void send();
    } else if (event.key === "Escape" && onCancel !== undefined) {
      event.preventDefault();
      onCancel();
    }
  };

  return (
    <form
      className="grid gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        void send();
      }}
    >
      <div className="relative">
        <Textarea
          ref={field}
          aria-label={label}
          value={text}
          rows={2}
          className="min-h-16"
          placeholder={copy.placeholder}
          disabled={disabled || sending}
          // A textarea that offers mentions as typed: ARIA's combobox, which no tag is.
          // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
          role="combobox"
          aria-expanded={open && choices.length > 0}
          aria-controls={listId}
          aria-autocomplete="list"
          {...(open && choices.length > 0
            ? { "aria-activedescendant": `${listId}-${current}` }
            : {})}
          onChange={(event) => {
            setText(event.target.value);
            setCaret(event.target.selectionStart);
            setHighlighted(0);
          }}
          onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
          onKeyDown={onKeyDown}
        />
        {open && choices.length > 0 && (
          <div
            id={listId}
            // Its options hold names with marks, which a datalist's cannot.
            // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
            role="listbox"
            aria-label={copy.mentionList}
            className="shadow-md absolute top-full left-0 z-20 mt-1 grid max-h-60 w-full overflow-y-auto rounded-md border border-border bg-popover p-1"
          >
            {choices.map((choice, index) => (
              <div
                key={`${choice.kind}:${choice.id}`}
                id={`${listId}-${index}`}
                // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
                role="option"
                // The field keeps focus; the option it points at is aria-activedescendant.
                tabIndex={-1}
                aria-selected={index === current}
                className="flex min-h-(--tap-sm) cursor-pointer items-center justify-between gap-2 rounded-sm px-2 text-sm aria-selected:bg-accent"
                onMouseDown={(event) => {
                  event.preventDefault();
                  pick(choice);
                }}
              >
                {choice.kind === "user" ? (
                  <>
                    <PersonLabel person={choice.person} />
                    {!choice.sees && (
                      <span className="shrink-0 text-xs text-muted-foreground">
                        {copy.cannotSee}
                      </span>
                    )}
                  </>
                ) : (
                  <span className="truncate">
                    {choice.label}
                    <span className="ml-2 text-xs text-muted-foreground">{copy.page}</span>
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
      <div className="flex flex-wrap justify-end gap-2">
        {onCancel !== undefined && (
          <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
            {copy.cancel}
          </Button>
        )}
        <Button type="submit" size="sm" disabled={disabled || sending || text.trim() === ""}>
          {sending ? copy.sending : submitLabel}
        </Button>
      </div>
    </form>
  );
}
