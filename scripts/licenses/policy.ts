/**
 * What each JS project may install beyond the allow list in deny.toml, and
 * where a package's metadata is wrong about its license. Every entry carries
 * its reason, and the check fails on an entry no installed package needs, so
 * the lists stay honest.
 */

/**
 * A package allowed outside the allow list because nothing of it ships: it is
 * used only while building, or never loaded at all. It is keyed by its exact
 * license, so a relicensed version fails again.
 */
export interface Exception {
  /** The package's name, or a name prefix ending in `*` for a package published once per platform. */
  name: string;
  /** The license expression the package declares, exactly. */
  license: string;
  /** Why no part of the package reaches the shipped files. */
  reason: string;
}

/**
 * The license of one package version whose package.json declares none, or
 * declares it wrongly, as its license file states it. A clarified license is
 * checked against the allow list like a declared one.
 */
export interface Clarification {
  name: string;
  /** The one version it covers: a new release is judged by its own metadata again. */
  version: string;
  /** The license expression the license file grants. */
  license: string;
  /** What the metadata gets wrong. */
  reason: string;
}

export interface Policy {
  /** Build-only exceptions to the allow list. */
  exceptions: readonly Exception[];
  clarifications: readonly Clarification[];
}

/** Each checked project, by its directory relative to the repository's root. */
export const POLICIES: Readonly<Record<string, Policy>> = {
  web: {
    exceptions: [
      {
        name: "caniuse-lite",
        license: "CC-BY-4.0",
        reason:
          "Browser support tables (with a small loader) read by browserslist, which " +
          "@babel/core loads for the TanStack Router plugin at build time. Never in the bundle.",
      },
    ],
    clarifications: [],
  },
  docs: {
    exceptions: [
      {
        name: "@img/sharp-libvips-*",
        license: "LGPL-3.0-or-later",
        reason:
          "libvips, the native library of sharp, which astro installs as an optional " +
          "dependency for its default image service. One package per platform. The docs " +
          "use the passthrough image service (astro.config.mjs), so sharp is never loaded, " +
          "and nothing of it reaches dist/.",
      },
      {
        name: "@fortawesome/fontawesome-free",
        license: "(CC-BY-4.0 AND OFL-1.1 AND MIT)",
        reason:
          "Font Awesome's icons, whose stylesheet mermaid-isomorphic adds to the page in the " +
          "headless Chromium that draws the diagrams while the docs build. The diagrams " +
          "reach dist/ as inline SVG; none of the package's fonts, styles or scripts do.",
      },
    ],
    clarifications: [
      {
        name: "khroma",
        version: "2.1.0",
        license: "MIT",
        reason: "package.json has no license field; the license file is the MIT License.",
      },
    ],
  },
  // The end-to-end tests, which ship nothing: every package must still be on the allow list.
  e2e: {
    exceptions: [],
    clarifications: [],
  },
};
