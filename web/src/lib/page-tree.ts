import type { TreePage } from "@/api/pages";
import { strings } from "./strings";

/** A tree of pages as the sidebar walks it: the pages at its top, and each page's own. */
export type PageForest = {
  roots: TreePage[];
  children: ReadonlyMap<string, TreePage[]>;
  byId: ReadonlyMap<string, TreePage>;
};

/**
 * Builds the tree from the server's list, in which each page comes after its
 * parent and siblings come in order. A page whose parent is not in the list,
 * such as one shared on its own, is at the top.
 */
export function forestOf(pages: readonly TreePage[]): PageForest {
  const byId = new Map(pages.map((page) => [page.id, page]));
  const children = new Map<string, TreePage[]>();
  const roots: TreePage[] = [];
  for (const page of pages) {
    const parent = page.parent_id ?? undefined;
    if (parent === undefined || !byId.has(parent)) {
      roots.push(page);
    } else {
      const siblings = children.get(parent);
      if (siblings === undefined) {
        children.set(parent, [page]);
      } else {
        siblings.push(page);
      }
    }
  }
  return { roots, children, byId };
}

/** The page and every page under it, as far as the tree holds them. */
export function subtreeOf(forest: PageForest, id: string): Set<string> {
  const found = new Set<string>([id]);
  const queue = [id];
  for (let next = queue.pop(); next !== undefined; next = queue.pop()) {
    for (const child of forest.children.get(next) ?? []) {
      if (!found.has(child.id)) {
        found.add(child.id);
        queue.push(child.id);
      }
    }
  }
  return found;
}

/** The pages beside one, itself included, in order; the top of the tree for a page at it. */
export function siblingsOf(forest: PageForest, page: TreePage): TreePage[] {
  const parent = page.parent_id ?? undefined;
  return parent !== undefined && forest.byId.has(parent)
    ? (forest.children.get(parent) ?? [])
    : forest.roots;
}

/** What a page is called where it is listed: its title, or Untitled. */
export function titleOf(page: { title: string }): string {
  return page.title.trim() === "" ? strings.pages.untitled : page.title;
}
