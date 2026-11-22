import { hueFor } from "@/lib/hue";
import { oklchToHex } from "@/lib/oklch";

/** Who is editing, as the page's other editors see them. */
export type Collaborator = {
  /** The account's id, so two tabs of one person count once. */
  id: string;
  name: string;
  /** Their hue, 0 to 359: their cursor and their dot are drawn in it. */
  hue: number;
  /**
   * Their hue as sRGB hex, for what y-prosemirror draws itself: the tint
   * over the text they select, which it writes as this colour plus an alpha.
   * Mid lightness and chroma, so it tints a light and a dark ground alike.
   */
  color: string;
};

/** The lightness and chroma of a selection's tint, which y-prosemirror lays at 44% alpha. */
const TINT = { lightness: 0.7, chroma: 0.14 } as const;

export function collaborator(id: string, name: string, hue: number): Collaborator {
  return { id, name, hue, color: oklchToHex(TINT.lightness, TINT.chroma, hue) };
}

/** What someone else's awareness state says of them, whatever it holds. */
export function collaboratorOf(user: unknown): Omit<Collaborator, "color"> | undefined {
  if (typeof user !== "object" || user === null) {
    return undefined;
  }
  const fields = user as Record<string, unknown>;
  const name = typeof fields["name"] === "string" ? fields["name"] : undefined;
  const id = typeof fields["id"] === "string" ? fields["id"] : name;
  if (name === undefined || id === undefined) {
    return undefined;
  }
  const hue =
    typeof fields["hue"] === "number" && Number.isInteger(fields["hue"])
      ? ((fields["hue"] % 360) + 360) % 360
      : hueFor(id);
  return { id, name, hue };
}

/**
 * Someone else's cursor: a caret and the name it carries, both in their hue.
 * It has the structure and classes of BlockNote's own, so BlockNote shows the
 * name while they type and hides it after, but its colour comes from the
 * theme (editor.css) through data-hue rather than from a fixed colour.
 */
export function renderCursor(user: { name: string; [key: string]: unknown }): HTMLElement {
  const who = collaboratorOf(user);
  const base = document.createElement("span");
  base.classList.add("bn-collaboration-cursor__base");
  base.dataset["hue"] = "";
  base.style.setProperty("--accent-h", String(who?.hue ?? hueFor(user.name)));

  const caret = document.createElement("span");
  caret.setAttribute("contenteditable", "false");
  caret.classList.add("bn-collaboration-cursor__caret");

  const label = document.createElement("span");
  label.classList.add("bn-collaboration-cursor__label");
  label.textContent = who?.name ?? user.name;

  caret.append(label);
  // Word joiners on both sides keep the caret from breaking a line on its own.
  base.append("\u2060", caret, "\u2060");
  return base;
}
