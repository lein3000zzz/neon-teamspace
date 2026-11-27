import { blockHasType } from "@blocknote/core";
import {
  DeleteLinkButton,
  EditLinkButton,
  LinkToolbar,
  useBlockNoteEditor,
  useComponentsContext,
  useDictionary,
  useEditorState,
  type LinkToolbarProps,
} from "@blocknote/react";
import { Download, ExternalLink } from "lucide-react";
import { openApart } from "@/lib/links";

/**
 * Opens a link of the editor's text that was clicked, in a tab cut off from
 * this one, in place of BlockNote's own opening, which leaves the new tab a
 * way back to this one. Always handles the click.
 */
export function openClickedLink(event: MouseEvent): boolean {
  const anchor = event.target instanceof Element ? event.target.closest("a") : null;
  if (anchor !== null && anchor.href !== "") {
    openApart(anchor.href);
  }
  return true;
}

/** BlockNote's link toolbar, its open button opening the link in a tab cut off from this one. */
export function LinkToolbarApart(props: LinkToolbarProps) {
  const Components = useComponentsContext();
  const dictionary = useDictionary();
  return (
    <LinkToolbar {...props}>
      <EditLinkButton {...props} />
      {Components !== undefined && (
        <Components.LinkToolbar.Button
          className="bn-button"
          mainTooltip={dictionary.link_toolbar.open.tooltip}
          label={dictionary.link_toolbar.open.tooltip}
          isSelected={false}
          onClick={() => openApart(props.url)}
          icon={<ExternalLink aria-hidden size={16} />}
        />
      )}
      <DeleteLinkButton {...props} />
    </LinkToolbar>
  );
}

/**
 * BlockNote's button that downloads the file of the block selected, opening it
 * in a tab cut off from this one.
 */
export function FileDownloadApart() {
  const Components = useComponentsContext();
  const dictionary = useDictionary();
  const editor = useBlockNoteEditor();
  const file = useEditorState({
    editor,
    selector: ({ editor: current }) => {
      const blocks = current.getSelection()?.blocks ?? [current.getTextCursorPosition().block];
      const only = blocks.length === 1 ? blocks[0] : undefined;
      if (only === undefined || !blockHasType(only, current, only.type, { url: "string" })) {
        return undefined;
      }
      return { type: only.type, url: only.props.url };
    },
  });
  if (file === undefined || Components === undefined) {
    return null;
  }
  const tooltips: Record<string, string | undefined> =
    dictionary.formatting_toolbar.file_download.tooltip;
  const label = tooltips[file.type] ?? tooltips["file"] ?? "";
  return (
    <Components.FormattingToolbar.Button
      className="bn-button"
      label={label}
      mainTooltip={label}
      icon={<Download aria-hidden size={18} />}
      onClick={() => {
        editor.focus();
        openApart(file.url);
      }}
    />
  );
}
