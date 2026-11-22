// Applies the stored theme before first paint. Loaded blocking from <head> as a
// same-origin file, so script-src 'self' runs it without 'unsafe-inline'.
// Mirrors src/theme/preferences.ts; boot-script.test.ts keeps the two in step.
(function () {
  "use strict";

  var defaults = { palette: "nothing", mode: "system", uiFont: "default" };
  // The accent where neither the user nor a workspace gives one.
  var fallbackHue = 250;
  var stored = {};
  var followed = null;

  try {
    var value = JSON.parse(window.localStorage.getItem("neon.theme"));
    if (value && typeof value === "object" && !Array.isArray(value)) {
      stored = value;
    }
  } catch {
    // Blocked storage or unparseable JSON: keep the defaults.
  }
  try {
    // The hue of the workspace last in view, the accent of someone without their own.
    followed = JSON.parse(window.localStorage.getItem("neon.theme.workspace-hue"));
  } catch {
    // None kept.
  }

  function oneOf(options, candidate, fallback) {
    return options.indexOf(candidate) === -1 ? fallback : candidate;
  }

  function isHue(candidate) {
    return (
      typeof candidate === "number" && candidate % 1 === 0 && candidate >= 0 && candidate < 360
    );
  }

  var hue = isHue(stored.accentHue) ? stored.accentHue : isHue(followed) ? followed : fallbackHue;
  var mode = oneOf(["system", "light", "dark"], stored.mode, defaults.mode);
  if (mode === "system") {
    var dark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
    mode = dark ? "dark" : "light";
  }

  var root = document.documentElement;
  root.setAttribute(
    "data-palette",
    oneOf(["nothing", "soft", "warm"], stored.palette, defaults.palette),
  );
  root.setAttribute("data-mode", mode);
  root.setAttribute("data-ui-font", oneOf(["default", "mono"], stored.uiFont, defaults.uiFont));
  root.style.setProperty("--accent-h", String(hue));
})();
