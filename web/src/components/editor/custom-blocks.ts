/**
 * The page editor's own blocks, beside BlockNote's defaults, as BlockNote block
 * configs: each type's name, its props with their defaults, and its content.
 * The config decides how a block is kept in the page's Yjs document, which
 * neon-blocks reads and writes on the server, so it is defined here once.
 *
 * The module imports nothing, so tools/blocknote-fixtures imports it too, by
 * its path, without this app's packages: it writes and checks the fixtures
 * that test neon-blocks with the same configs the editor has. Each project
 * makes its block specs from them with its own BlockNote: the editor draws the
 * blocks (database-block.tsx), and the fixtures tool only converts them.
 */

/**
 * A database shown in the page: the database's id, empty for a block not given
 * one yet, and the id of the view it is shown in, when the block names one.
 * It has no content of its own.
 */
export const databaseBlockConfig = {
  type: "database",
  propSchema: {
    databaseId: { default: "" },
    viewId: { default: undefined, type: "string" },
  },
  content: "none",
} as const;
