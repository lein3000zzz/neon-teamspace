import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { showPage, useUpdatePage, type Page } from "@/api/pages";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Button } from "@/components/ui/button";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";

const copy = strings.pages.page;

/** How long typing pauses before the title is saved. */
export const TITLE_SAVE_MS = 500;
/** The longest title the server takes. */
const MAX_TITLE = 500;

const titleStyle =
  "text-3xl leading-tight font-bold break-words [font-family:var(--page-font)] sm:text-[2.5rem]";

type PageTitleProps = {
  workspaceId: string;
  page: Page;
  editable: boolean;
  /** Enter in the title: on to the page's content. */
  onEnter: () => void;
};

/**
 * The page's title. For those who edit it, it is typed in place: the sidebar
 * and the way down to the page show it as it is typed, and it is saved once
 * typing pauses, or as the field is left. An empty page opens with the cursor
 * in it.
 */
export function PageTitle({ workspaceId, page, editable, onEnter }: PageTitleProps) {
  if (!editable) {
    return (
      <h1 className={titleStyle} style={{ paddingInline: "var(--page-inset)" }}>
        {page.title.trim() === "" ? (
          <span className="text-muted-foreground">{strings.pages.untitled}</span>
        ) : (
          page.title
        )}
      </h1>
    );
  }
  return <TitleField workspaceId={workspaceId} page={page} onEnter={onEnter} />;
}

function TitleField({ workspaceId, page, onEnter }: Omit<PageTitleProps, "editable">) {
  const queryClient = useQueryClient();
  const update = useUpdatePage(workspaceId, page.id);
  const field = useRef<HTMLTextAreaElement>(null);
  const [value, setValue] = useState(page.title);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  /** The title last sent, so leaving the field does not send it again. */
  const sent = useRef(page.title);
  const save = useRef(update.mutate);
  const emptyAtOpen = useRef(page.title === "");

  useEffect(() => {
    save.current = update.mutate;
  }, [update.mutate]);

  useEffect(() => {
    if (emptyAtOpen.current) {
      field.current?.focus();
    }
  }, []);

  /** Saves what is typed now, unless it was sent already; `again` sends it anyway. */
  const flush = (again = false) => {
    clearTimeout(timer.current);
    timer.current = undefined;
    const text = field.current?.value ?? value;
    if (again || text !== sent.current) {
      sent.current = text;
      save.current({ title: text });
    }
  };

  // Typed but not yet saved when the page closes: saved on the way out.
  useEffect(
    () => () => {
      if (timer.current !== undefined) {
        clearTimeout(timer.current);
        const text = field.current?.value;
        if (text !== undefined && text !== sent.current) {
          save.current({ title: text });
        }
      }
    },
    [],
  );

  // A title changed elsewhere shows here, unless it is being typed.
  useEffect(() => {
    if (document.activeElement !== field.current && timer.current === undefined) {
      setValue(page.title);
      sent.current = page.title;
    }
  }, [page.title]);

  // The field grows with the title, which wraps rather than scrolls.
  useLayoutEffect(() => {
    const element = field.current;
    if (element !== null) {
      element.style.height = "auto";
      element.style.height = `${element.scrollHeight}px`;
    }
  });

  return (
    <div className="grid gap-2">
      <h1 className={titleStyle}>
        <textarea
          ref={field}
          aria-label={copy.title}
          rows={1}
          maxLength={MAX_TITLE}
          placeholder={strings.pages.untitled}
          value={value}
          onChange={(event) => {
            const text = event.target.value.replaceAll(/[\r\n]+/g, " ");
            setValue(text);
            showPage(queryClient, workspaceId, page.id, { title: text });
            clearTimeout(timer.current);
            timer.current = setTimeout(() => flush(), TITLE_SAVE_MS);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              flush();
              onEnter();
            }
          }}
          onBlur={() => flush()}
          style={{ paddingInline: "var(--page-inset)" }}
          className="block w-full resize-none overflow-hidden bg-transparent outline-none placeholder:text-muted-foreground focus-visible:shadow-none focus-visible:outline-none"
        />
      </h1>
      {update.isError && (
        <ErrorNotice error={update.error} title={copy.saveFailed}>
          <div>
            <Button variant="outline" size="sm" onClick={() => flush(true)}>
              {copy.retry}
            </Button>
          </div>
        </ErrorNotice>
      )}
    </div>
  );
}

/** What the browser tab says for a page. */
export function documentTitle(page: Pick<Page, "title" | "icon">): string {
  const icon = page.icon?.trim();
  return `${icon ? `${icon} ` : ""}${titleOf(page)} · ${strings.product}`;
}
