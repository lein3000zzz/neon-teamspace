import { createReactInlineContentSpec } from "@blocknote/react";
import { Link } from "@tanstack/react-router";
import { HueDot } from "@/components/nodes/hue-dot";
import { hueFor } from "@/lib/hue";
import { attribution } from "@/lib/people";
import { strings } from "@/lib/strings";
import { mentionInlineConfig } from "./custom-blocks";
import { useEditorPage } from "./editor-page";

/**
 * A mention as the editor draws it: someone in their hue, by the name they
 * have now when the user may know it, an agent "via" the person it acts for,
 * or a page, a link to it; the label it was written with otherwise.
 */
export const mentionInline = createReactInlineContentSpec(mentionInlineConfig, {
  render: function Mention({ inlineContent }) {
    const page = useEditorPage();
    const { kind, target, label } = inlineContent.props;
    if (kind === "page") {
      return (
        <Link
          to="/w/$slug/p/$pageId"
          params={{ slug: page.slug, pageId: target }}
          className="mention mention-page"
          data-mention="page"
        >
          @{label === "" ? strings.pages.untitled : label}
        </Link>
      );
    }
    const person = page.people.get(target);
    const name = person === undefined ? label : attribution(person);
    return (
      <span className="mention" data-mention="user" data-target={target}>
        <HueDot hue={person?.hue ?? hueFor(target)} className="size-1.5" />@{name}
      </span>
    );
  },
});
