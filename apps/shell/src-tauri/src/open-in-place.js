// iOS: the webview there has no new windows, and drops what window.open asks for. The page
// loads in the window instead, as Android's single window does, so the origin lock decides
// where it goes: the server's own pages in the window, another origin's in the browser. The
// call returns null, as window.open with noopener does.
(() => {
  const OPENABLE = new Set(["http:", "https:", "mailto:", "tel:", "blob:"]);
  const openInPlace = (url) => {
    if (url === undefined || url === null || String(url) === "") {
      return null;
    }
    let target;
    try {
      target = new URL(String(url), window.location.href);
    } catch {
      return null;
    }
    if (OPENABLE.has(target.protocol)) {
      window.location.assign(target.href);
    }
    return null;
  };
  Object.defineProperty(window, "open", {
    value: openInPlace,
    writable: false,
    configurable: false,
  });
})();
