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

/** The longest name a cursor carries, as the server takes display names; a uuid's length. */
const NAME_LIMIT = 100;
const ID_LIMIT = 36;

export function collaborator(id: string, name: string, hue: number): Collaborator {
  return { id, name, hue, color: oklchToHex(TINT.lightness, TINT.chroma, hue) };
}

/** What someone else's awareness state says of them, whatever it holds. */
export function collaboratorOf(user: unknown): Omit<Collaborator, "color"> | undefined {
  if (!isRecord(user)) {
    return undefined;
  }
  const name = user["name"];
  const id = user["id"];
  if (typeof name !== "string" || typeof id !== "string" || id === "" || id.length > ID_LIMIT) {
    return undefined;
  }
  const hue =
    typeof user["hue"] === "number" && Number.isInteger(user["hue"])
      ? ((user["hue"] % 360) + 360) % 360
      : hueFor(id);
  return { id, name: [...name].slice(0, NAME_LIMIT).join(""), hue };
}

/**
 * Someone else's awareness state as the editor may draw it: who they are,
 * with their selection's colour made here from their hue, and the two ends of
 * their cursor in the form Yjs writes them. Nothing else in it is kept, so no
 * value another client made up reaches the page: y-prosemirror writes the
 * colour into a style attribute as it finds it, and reads the cursor without
 * checking it. A state of `null`, a cursor that left, stays `null`.
 */
export function cleanState(state: unknown): Record<string, unknown> | null {
  if (state === null) {
    return null;
  }
  if (!isRecord(state)) {
    return {};
  }
  const clean: Record<string, unknown> = {};
  const who = collaboratorOf(state["user"]);
  if (who !== undefined) {
    clean["user"] = collaborator(who.id, who.name, who.hue);
  }
  const cursor = state["cursor"];
  if (cursor === null) {
    clean["cursor"] = null;
  } else if (isRecord(cursor)) {
    const anchor = cleanPosition(cursor["anchor"]);
    const head = cleanPosition(cursor["head"]);
    if (anchor !== undefined && head !== undefined) {
      clean["cursor"] = { anchor, head };
    }
  }
  return clean;
}

type Id = { client: number; clock: number };

/** A relative position as `Y.relativePositionToJSON` writes one, or undefined for anything else. */
function cleanPosition(
  position: unknown,
): { type?: Id; tname?: string; item?: Id; assoc?: number } | undefined {
  if (!isRecord(position)) {
    return undefined;
  }
  const clean: { type?: Id; tname?: string; item?: Id; assoc?: number } = {};
  for (const key of ["type", "item"] as const) {
    const value = position[key];
    if (value === undefined || value === null) {
      continue;
    }
    const id = cleanId(value);
    if (id === undefined) {
      return undefined;
    }
    clean[key] = id;
  }
  const tname = position["tname"];
  if (typeof tname === "string") {
    clean.tname = tname;
  } else if (tname !== undefined && tname !== null) {
    return undefined;
  }
  const assoc = position["assoc"];
  if (Number.isSafeInteger(assoc)) {
    clean.assoc = assoc as number;
  }
  return clean;
}

function cleanId(id: unknown): Id | undefined {
  if (!isRecord(id)) {
    return undefined;
  }
  const { client, clock } = id;
  return isCount(client) && isCount(clock) ? { client, clock } : undefined;
}

function isCount(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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
  base.append("⁠", caret, "⁠");
  return base;
}
