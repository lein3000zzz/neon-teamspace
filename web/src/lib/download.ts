/** How long a saved file's address stays, for browsers that start the download late. */
const KEPT_MS = 60 * 1000;

/**
 * Saves a file the app holds as a download named `name`, the way a link to
 * it with `download` would: the browser asks where to put it, or puts it in
 * its downloads.
 */
export function saveFile(file: Blob, name: string): void {
  const address = URL.createObjectURL(file);
  const link = document.createElement("a");
  link.href = address;
  link.download = name;
  link.hidden = true;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(address), KEPT_MS);
}
