// Applies the stored theme before first paint. Loaded blocking from <head> as a
// same-origin file, so script-src 'self' runs it without 'unsafe-inline'.
// Mirrors src/theme/preferences.ts; boot-script.test.ts keeps the two in step.
(function () {
  "use strict";

  var defaults = { palette: "nothing", mode: "system", accentHue: 250, uiFont: "default" };
  var stored = {};

  try {
    var value = JSON.parse(window.localStorage.getItem("neon.theme"));
    if (value && typeof value === "object" && !Array.isArray(value)) {
      stored = value;
    }
  } catch {
    // Blocked storage or unparseable JSON: keep the defaults.
  }

  function oneOf(options, candidate, fallback) {
    return options.indexOf(candidate) === -1 ? fallback : candidate;
  }

  var hue = stored.accentHue;
  var validHue = typeof hue === "number" && hue % 1 === 0 && hue >= 0 && hue < 360;
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
  root.style.setProperty("--accent-h", String(validHue ? hue : defaults.accentHue));
})();
