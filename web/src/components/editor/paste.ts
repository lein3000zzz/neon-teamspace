/**
 * The most a paste may add to a page, as estimated below. The server takes a
 * message of up to 4 MiB by default, and a paste goes to it as one change, in
 * one message; past that it closes the connection, and the paste is lost. The
 * margin is for what the estimate misses, such as a list's nesting.
 */
export const PASTE_LIMIT = 3 * 1024 * 1024;

/** What a block costs in a page's Yjs update besides its text: its id, its type and its props. */
const BLOCK_BYTES = 220;

const encoder = new TextEncoder();

/**
 * Whether what is on the clipboard fits in one change the server takes. Its
 * plain text stands for it, as the editor makes blocks of the text whatever
 * markup comes with it: its size in UTF-8, as Yjs sends text, and a block's
 * cost for every line that holds anything, as each may become a block.
 */
export function pasteFits(data: Pick<DataTransfer, "getData">): boolean {
  const text =
    data.getData("text/plain") || data.getData("text/markdown") || data.getData("text/html");
  if (text.length * 3 + text.length * BLOCK_BYTES <= PASTE_LIMIT) {
    return true;
  }
  return encoder.encode(text).byteLength + filledLines(text) * BLOCK_BYTES <= PASTE_LIMIT;
}

function filledLines(text: string): number {
  let count = 0;
  let start = 0;
  while (start <= text.length) {
    const end = text.indexOf("\n", start);
    const stop = end === -1 ? text.length : end;
    if (text.slice(start, stop).trim() !== "") {
      count += 1;
    }
    start = stop + 1;
  }
  return count;
}
