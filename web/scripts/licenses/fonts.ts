/**
 * Font files the app serves from public/. They are not packages, so the
 * package check never sees them, and deny.toml lists code licenses only. A
 * font is accepted under the SIL Open Font License 1.1, and only with that
 * license's text in the same directory, where the OFL requires it to travel
 * with the files.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join, posix } from "node:path";

/** The one license accepted for fonts, as an SPDX id. */
export const FONT_LICENSE = "OFL-1.1";

const FONT_FILE = /\.(?:woff2?|ttf|otf)$/i;
const LICENSE_FILE = /^(?:licen[cs]e|copying|ofl)\b/i;
const OFL_1_1 = /SIL OPEN FONT LICENSE\s+Version 1\.1/i;

export interface BundledFont {
  /** The directory holding the font files, relative to public/ and with forward slashes. */
  dir: string;
  fonts: string[];
  /** License files in that directory, as paths relative to public/. */
  licenseFiles: string[];
}

/** Every directory under `publicDir` that holds font files, at any depth. */
export function bundledFonts(publicDir: string): BundledFont[] {
  const files = readdirSync(publicDir, { recursive: true, encoding: "utf8" }).map((file) =>
    file.split("\\").join("/"),
  );
  const dirs = new Set(
    files.filter((file) => FONT_FILE.test(file)).map((file) => posix.dirname(file)),
  );
  return [...dirs].toSorted().map((dir) => {
    const inDir = files.filter((file) => posix.dirname(file) === dir);
    return {
      dir,
      fonts: inDir.filter((file) => FONT_FILE.test(file)).map((file) => posix.basename(file)),
      licenseFiles: inDir.filter((file) => LICENSE_FILE.test(posix.basename(file))).toSorted(),
    };
  });
}

/** The license texts shipped with a font directory, joined. */
export function fontLicenseText(publicDir: string, font: BundledFont): string {
  return font.licenseFiles
    .map((file) => readFileSync(join(publicDir, file), "utf8").trim())
    .join("\n\n");
}

/** Why a font directory may not ship, or null when it may. */
export function fontProblem(publicDir: string, font: BundledFont): string | null {
  if (font.licenseFiles.length === 0) {
    return "no license file next to the font files";
  }
  if (!OFL_1_1.test(fontLicenseText(publicDir, font))) {
    return `license is not the SIL Open Font License 1.1 (${FONT_LICENSE})`;
  }
  return null;
}
