/**
 * Sends the browser away from the app, to an address of another program: an
 * OAuth client's redirect URI, which may be another site, this machine, or a
 * scheme of an app's own. Kept apart so that tests can stand in for it.
 */
export function leaveFor(address: string) {
  window.location.assign(address);
}
