import { createReactBlockSpec } from "@blocknote/react";
import { DatabaseEmbed } from "@/components/databases/database-embed";
import { databaseBlockConfig } from "./custom-blocks";
import { useEditorPage } from "./editor-page";

/**
 * The database block as the editor draws it: the database it names, shown in
 * its views inside the page, or, while it names none, the way to make one.
 * The view it shows is kept in the block, for everyone, when the user edits
 * the page.
 */
export const databaseBlock = createReactBlockSpec(databaseBlockConfig, {
  render: function DatabaseBlock({ block, editor }) {
    const page = useEditorPage();
    return (
      <DatabaseEmbed
        databaseId={block.props.databaseId}
        viewId={block.props.viewId}
        creating={page.creating.has(block.id)}
        onCreate={page.editable && page.holdsDatabases ? () => page.createIn(block.id) : undefined}
        onViewChange={
          page.editable
            ? (viewId) => {
                if (editor.getBlock(block.id) !== undefined) {
                  editor.updateBlock(block.id, { props: { viewId } });
                }
              }
            : undefined
        }
      />
    );
  },
});
