import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import type { InlineRun } from "@/api/comments";
import { HueDot } from "@/components/nodes/hue-dot";
import { hueFor } from "@/lib/hue";
import { attribution, type Someone } from "@/lib/people";
import { strings } from "@/lib/strings";

type Styles = Record<string, boolean | string> | undefined;

/** Text in its styles, each a tag, line breaks kept. */
function styled(text: string, styles: Styles, key: string): ReactNode {
  let node: ReactNode = text;
  if (styles?.["code"] === true) {
    return <code key={key}>{text}</code>;
  }
  if (styles?.["bold"] === true) {
    node = <strong>{node}</strong>;
  }
  if (styles?.["italic"] === true) {
    node = <em>{node}</em>;
  }
  if (styles?.["underline"] === true) {
    node = <u>{node}</u>;
  }
  if (styles?.["strike"] === true) {
    node = <s>{node}</s>;
  }
  return <span key={key}>{node}</span>;
}

/** Only web links are followed; anything else shows as text. */
function safeHref(href: string): string | undefined {
  try {
    const url = new URL(href, window.location.origin);
    return url.protocol === "https:" || url.protocol === "http:" || url.protocol === "mailto:"
      ? url.href
      : undefined;
  } catch {
    return undefined;
  }
}

type CommentContentProps = {
  runs: readonly InlineRun[];
  slug: string;
  /** Everyone a mention may name, by id; empty for a guest. */
  people: ReadonlyMap<string, Someone>;
};

/**
 * A comment's inline content: its text with styles, its links, and its
 * mentions, each drawn as the editor draws one.
 */
export function CommentContent({ runs, slug, people }: CommentContentProps) {
  return (
    <p className="text-sm break-words whitespace-pre-wrap">
      {runs.map((run, index) => {
        const key = String(index);
        switch (run.type) {
          case "text":
            return styled(run.text, run.styles, key);
          case "link": {
            const href = safeHref(run.href);
            const inner = run.content.map((part, at) =>
              styled(part.text, part.styles, `${key}-${at}`),
            );
            return href === undefined ? (
              <span key={key}>{inner}</span>
            ) : (
              <a
                key={key}
                href={href}
                target="_blank"
                rel="noreferrer noopener"
                className="text-primary underline underline-offset-4"
              >
                {inner}
              </a>
            );
          }
          case "mention": {
            const { kind, target, label } = run.props;
            if (kind === "page") {
              return (
                <Link
                  key={key}
                  to="/w/$slug/p/$pageId"
                  params={{ slug, pageId: target }}
                  className="mention mention-page"
                >
                  @{label === "" ? strings.pages.untitled : label}
                </Link>
              );
            }
            const person = people.get(target);
            return (
              <span key={key} className="mention">
                <HueDot hue={person?.hue ?? hueFor(target)} className="size-1.5" />@
                {person === undefined ? label : attribution(person)}
              </span>
            );
          }
        }
      })}
    </p>
  );
}
