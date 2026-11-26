import { BlockNoteSchema } from "@blocknote/core";
import { databaseBlockConfig, mentionInlineConfig } from "./custom-blocks";
import { databaseBlock } from "./database-block";
import { mentionInline } from "./mention";

/**
 * The page editor's schema: BlockNote's default blocks and inline content and
 * the app's own, each under its type's name. tools/blocknote-fixtures makes
 * the same schema from the configs in custom-blocks.ts.
 */
export const schema = BlockNoteSchema.create().extend({
  blockSpecs: { [databaseBlockConfig.type]: databaseBlock() },
  inlineContentSpecs: { [mentionInlineConfig.type]: mentionInline },
});
