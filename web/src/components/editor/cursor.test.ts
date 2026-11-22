import { describe, expect, it } from "vitest";
import { hueFor } from "@/lib/hue";
import { collaborator, collaboratorOf, renderCursor } from "./cursor";

describe("collaboration cursors", () => {
  it("draws someone's caret and name in their hue, through the theme", () => {
    const cursor = renderCursor({ name: "Bob", id: "bob", hue: 177, color: "#000000" });

    expect(cursor).toHaveClass("bn-collaboration-cursor__base");
    expect(cursor).toHaveAttribute("data-hue", "");
    expect(cursor.style.getPropertyValue("--accent-h")).toBe("177");
    expect(cursor.querySelector(".bn-collaboration-cursor__label")).toHaveTextContent("Bob");
    // No colour of its own: editor.css draws it from --neon.
    expect(
      cursor.querySelector(".bn-collaboration-cursor__caret")?.getAttribute("style"),
    ).toBeNull();
  });

  it("takes the hue from the id when a state carries none", () => {
    expect(collaboratorOf({ id: "0193a5b2-0000-7000-8000-00000000c001", name: "Bob" })).toEqual({
      id: "0193a5b2-0000-7000-8000-00000000c001",
      name: "Bob",
      hue: hueFor("0193a5b2-0000-7000-8000-00000000c001"),
    });
    expect(collaboratorOf({ name: 3 })).toBeUndefined();
    expect(collaboratorOf(null)).toBeUndefined();
  });

  it("gives y-prosemirror a six-digit colour for the selection's tint", () => {
    expect(collaborator("ada", "Ada", 342).color).toMatch(/^#[0-9a-f]{6}$/);
  });
});
