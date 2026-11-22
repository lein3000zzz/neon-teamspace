import { describe, expect, it } from "vitest";
import type { TreePage } from "@/api/pages";
import { forestOf, siblingsOf, subtreeOf, titleOf } from "./page-tree";

const entry = (id: string, parent?: string): TreePage => ({
  id,
  title: id,
  child_count: 0,
  access: "edit",
  ...(parent === undefined ? {} : { parent_id: parent }),
});

describe("page trees", () => {
  // a > (b > d), c; e's parent is not in the list, as for a page shared on its own.
  const forest = forestOf([
    entry("a"),
    entry("b", "a"),
    entry("d", "b"),
    entry("c", "a"),
    entry("e", "x"),
  ]);

  it("puts pages under their parents, in order, and those without one at the top", () => {
    expect(forest.roots.map((page) => page.id)).toEqual(["a", "e"]);
    expect(forest.children.get("a")?.map((page) => page.id)).toEqual(["b", "c"]);
  });

  it("finds a page's subtree and its siblings", () => {
    expect([...subtreeOf(forest, "a")].toSorted()).toEqual(["a", "b", "c", "d"]);
    expect([...subtreeOf(forest, "c")]).toEqual(["c"]);
    const c = forest.byId.get("c");
    const e = forest.byId.get("e");
    expect(c && siblingsOf(forest, c).map((page) => page.id)).toEqual(["b", "c"]);
    expect(e && siblingsOf(forest, e).map((page) => page.id)).toEqual(["a", "e"]);
  });

  it("calls a page without a title Untitled", () => {
    expect(titleOf({ title: "  " })).toBe("Untitled");
    expect(titleOf({ title: "Roadmap" })).toBe("Roadmap");
  });
});
