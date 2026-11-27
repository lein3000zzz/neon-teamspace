import { describe, expect, it } from "vitest";
import type { Agent } from "@/api/agents";
import type { Member } from "@/api/workspaces";
import { actsForOwner, assignable, attribution, attributionInSentence, peopleById } from "./people";

const ada = { id: "a", kind: "human", display_name: "Ada", hue: 1 } as const;
const owner = { id: "a", display_name: "Ada", hue: 1 };

const agent = (id: string, status: Agent["status"], kind: Agent["kind"] = "agent"): Agent => ({
  id,
  kind,
  workspace_id: "w",
  name: id === "s" ? "Scout" : "Hook",
  hue: 2,
  owner,
  status,
  created_at: "2026-11-20T09:00:00Z",
  updated_at: "2026-11-20T09:00:00Z",
});

describe("naming people and agents", () => {
  it("names a person by name, an agent with whom it acts for", () => {
    expect(attribution(ada)).toBe("Ada");
    expect(attribution({ ...ada, id: "s", kind: "agent", display_name: "Scout", owner })).toBe(
      "Scout · via Ada",
    );
    expect(attribution({ id: "s", kind: "agent", display_name: "Scout", hue: 2 })).toBe("Scout");
    expect(
      attribution({ id: "s", kind: "agent", display_name: "Scout", hue: 2, owner, deleted: true }),
    ).toBe("Scout · via Ada (deleted)");
    // Inside a sentence, as notification mail says it.
    expect(
      attributionInSentence({ ...ada, id: "s", kind: "agent", display_name: "Scout", owner }),
    ).toBe("Scout via Ada");
    expect(attributionInSentence(ada)).toBe("Ada");
    expect(actsForOwner(ada)).toBe(false);
    expect(actsForOwner({ kind: "integration" })).toBe(true);
  });

  it("gives work to people and live agents, never integrations", () => {
    expect(assignable(ada)).toBe(true);
    expect(assignable({ ...ada, kind: "agent" })).toBe(true);
    expect(assignable({ ...ada, kind: "integration" })).toBe(false);
    expect(assignable({ ...ada, kind: "agent", deleted: true })).toBe(false);
  });

  it("finds members, and agents deleted since", () => {
    const members: Member[] = [
      { user: ada, role: "owner", joined_at: "2026-11-20T09:00:00Z", suspended: false },
      {
        user: { id: "s", kind: "agent", display_name: "Scout", hue: 2, owner },
        role: "member",
        joined_at: "2026-11-20T09:00:00Z",
        suspended: true,
      },
    ];
    const people = peopleById(members, [
      agent("s", "suspended"),
      agent("h", "deleted", "integration"),
    ]);

    expect([...people.keys()].toSorted()).toEqual(["a", "h", "s"]);
    expect(people.get("s")?.deleted).toBeUndefined();
    expect(people.get("h")).toEqual({
      id: "h",
      kind: "integration",
      display_name: "Hook",
      hue: 2,
      owner,
      deleted: true,
    });
  });
});
