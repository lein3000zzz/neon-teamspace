import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { fakePages, PAGE_IDS } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { fakeWorkspaces } from "@/test/workspaces";

/** The parts of the home page below its header, in their order, by their headings. */
function parts(first: HTMLElement): string[] {
  return [...(first.parentElement?.children ?? [])].map(
    (part) => part.querySelector("h2")?.textContent ?? "",
  );
}

describe("a workspace's home", () => {
  it("opens on the teamspaces when nothing is a favorite, leaving no room for favorites", async () => {
    fakeWorkspaces();
    fakePages();
    renderApp("/w/design");

    const teamspaces = await screen.findByRole("region", { name: "Your teamspaces" });
    expect(await within(teamspaces).findByRole("link", { name: /General/ })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Favorites" })).toBeNull();
    // The first part under the header, which drops its rule.
    expect(parts(teamspaces)[0]).toBe("Your teamspaces");
    expect(teamspaces.parentElement?.contains(screen.getByRole("heading", { level: 1 }))).toBe(
      false,
    );
  });

  it("opens on the favorites when there are some", async () => {
    fakeWorkspaces();
    fakePages({ favorites: [PAGE_IDS.roadmap] });
    renderApp("/w/design");

    const favorites = await screen.findByRole("region", { name: "Favorites" });
    expect(await within(favorites).findByText("Roadmap")).toBeInTheDocument();
    expect(parts(favorites)).toEqual(["Favorites", "Your teamspaces"]);
  });
});
