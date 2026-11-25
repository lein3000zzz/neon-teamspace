import { describe, expect, it } from "vitest";
import { hueFor } from "@/lib/hue";
import {
  cleanState,
  collaborator,
  collaboratorLabel,
  collaboratorOf,
  renderCursor,
} from "./cursor";

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

  it("keeps of another's awareness only who they are and where their cursor is", () => {
    const id = "0193a5b2-0000-7000-8000-00000000c001";
    const cursor = {
      anchor: {
        type: { client: 7, clock: 2 },
        tname: null,
        item: { client: 7, clock: 4 },
        assoc: 0,
      },
      head: { item: { client: 7, clock: 9 } },
    };
    const clean = cleanState({
      user: {
        id,
        name: "Bob",
        hue: 177,
        color: "#000000; position: fixed; inset: 0",
        avatar: "javascript:alert(1)",
      },
      cursor,
      extra: { anything: true },
    });

    expect(clean).toEqual({
      user: collaborator(id, "Bob", 177),
      cursor: {
        anchor: { type: { client: 7, clock: 2 }, item: { client: 7, clock: 4 }, assoc: 0 },
        head: { item: { client: 7, clock: 9 } },
      },
    });
  });

  it("drops a cursor Yjs could not read, and keeps one that left", () => {
    const user = { id: "bob", name: "Bob", hue: 3 };
    expect(cleanState({ user, cursor: {} })).toEqual({ user: collaborator("bob", "Bob", 3) });
    expect(
      cleanState({ user, cursor: { anchor: { item: { client: "7" } }, head: {} } }),
    ).not.toHaveProperty("cursor");
    expect(cleanState({ user, cursor: null })).toHaveProperty("cursor", null);
    expect(cleanState(null)).toBeNull();
    expect(cleanState("Bob")).toEqual({});
  });

  it("cuts a name to the length display names have", () => {
    const clean = cleanState({ user: { id: "bob", name: "B".repeat(5000), hue: 3 } });
    expect(clean).toHaveProperty("user.name", "B".repeat(100));
    expect(cleanState({ user: { id: "x".repeat(37), name: "Bob" } })).toEqual({});
  });

  it("names an agent with the person it acts for", () => {
    const id = "0193a5b2-0000-7000-8000-00000000c0a1";
    const scout = { id, name: "Scout", hue: 200, kind: "agent" as const, owner: "Ada" };

    expect(collaboratorOf(scout)).toEqual(scout);
    expect(cleanState({ user: scout })).toEqual({
      user: { ...collaborator(id, "Scout", 200), kind: "agent", owner: "Ada" },
    });
    expect(
      renderCursor({ ...scout, color: "#000000" }).querySelector(".bn-collaboration-cursor__label"),
    ).toHaveTextContent("Scout · via Ada");
    expect(collaboratorLabel(scout)).toBe("Scout · via Ada");
    expect(collaboratorLabel({ name: "Hook", kind: "integration" })).toBe("Hook");
    expect(collaboratorLabel({ name: "Bob" })).toBe("Bob");
  });

  it("keeps a person a person whatever kind a state claims", () => {
    const user = { id: "bob", name: "Bob", hue: 3 };
    expect(collaboratorOf({ ...user, kind: "robot", owner: "Ada" })).toEqual(user);
    expect(collaboratorOf({ ...user, kind: "human", owner: "Ada" })).toEqual(user);
    expect(collaboratorOf({ ...user, kind: "agent", owner: 7 })).toEqual({
      ...user,
      kind: "agent",
    });
    expect(collaboratorOf({ ...user, kind: "agent", owner: "A".repeat(500) })).toHaveProperty(
      "owner",
      "A".repeat(100),
    );
  });
});
