// Applies the palette and the system's light or dark mode before first paint. Loaded blocking
// from <head> as a file of the app's own, so the policy's script-src 'self' runs it.
(function () {
  "use strict";

  var palette = "nothing";
  try {
    var stored = window.localStorage.getItem("neon.shell.palette");
    if (stored === "soft" || stored === "warm") {
      palette = stored;
    }
  } catch {
    // Blocked storage: the default palette.
  }
  var dark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
  var root = document.documentElement;
  root.setAttribute("data-palette", palette);
  root.setAttribute("data-mode", dark ? "dark" : "light");
})();
