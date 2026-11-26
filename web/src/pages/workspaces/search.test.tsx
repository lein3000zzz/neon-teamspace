import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderApp } from "@/test/render-app";
import { fakePages, PAGE_IDS, samplePage, samplePages } from "@/test/pages";
import { fakeSearch, result } from "@/test/search";
import { fakeWorkspaces, people, TEAMSPACE_IDS } from "@/test/workspaces";

const pages = samplePages();
const byId = samplePage;

async function results() {
  return within(await screen.findByRole("region", { name: "Results" }));
}

describe("the search page", () => {
  it("searches what the address says, and follows what is typed", async () => {
    fakeWorkspaces();
    fakePages();
    const api = fakeSearch({
      answers: {
        plan: [result(byId(PAGE_IDS.roadmap), "", "the quarterly plan")],
        goals: [result(byId(PAGE_IDS.goals), "Goals", "")],
      },
    });
    const { user, router } = renderApp("/w/design/search?q=plan");

    const found = await results();
    const link = await found.findByRole("link", { name: /Roadmap/ });
    expect(link).toHaveAttribute("href", `/w/design/p/${PAGE_IDS.roadmap}`);
    expect(found.getByText("the quarterly plan").tagName).toBe("MARK");
    expect(api.asked.at(-1)).toEqual({ query: "plan", limit: "20" });

    const field = screen.getByRole("searchbox", { name: "Search" });
    expect(field).toHaveValue("plan");
    await user.clear(field);
    await user.type(field, "goals");
    expect(await (await results()).findByRole("link", { name: /Goals/ })).toBeInTheDocument();
    await waitFor(() => expect(router.state.location.search).toEqual({ q: "goals" }));
  });

  it("narrows by the filters, which the address keeps", async () => {
    fakeWorkspaces();
    fakePages();
    const api = fakeSearch({ answers: { plan: [result(byId(PAGE_IDS.launch), "", "")] } });
    const { user, router } = renderApp("/w/design/search?q=plan");
    await (await results()).findByRole("link", { name: /Launch/ });

    const filters = within(screen.getByRole("region", { name: "Filters" }));
    await user.selectOptions(
      await filters.findByRole("combobox", { name: "Teamspace" }),
      "General",
    );
    await user.selectOptions(filters.getByRole("combobox", { name: "Kind" }), "Rows");
    await user.selectOptions(filters.getByRole("combobox", { name: "Created by" }), "Cy");
    await user.selectOptions(filters.getByRole("combobox", { name: "Last edited by" }), "Bob");
    await user.selectOptions(filters.getByRole("combobox", { name: "Edited" }), "In the last week");

    await waitFor(() =>
      expect(api.asked.at(-1)).toEqual({
        query: "plan",
        teamspace_id: TEAMSPACE_IDS.general,
        kind: "row",
        created_by: people.cy.id,
        edited_by: people.bob.id,
        edited_within: "7d",
        limit: "20",
      }),
    );
    expect(router.state.location.search).toEqual({
      q: "plan",
      teamspace: TEAMSPACE_IDS.general,
      kind: "row",
      by: people.cy.id,
      editor: people.bob.id,
      within: "7d",
    });
    // Back to any kind.
    await user.selectOptions(
      filters.getByRole("combobox", { name: "Kind" }),
      "Pages, databases and rows",
    );
    await waitFor(() => expect(api.asked.at(-1)?.["kind"]).toBeUndefined());
  });

  it("reads more results a part at a time, and says when nothing matches", async () => {
    fakeWorkspaces();
    fakePages();
    const many = pages.slice(0, 5).map((page) => result(page, "", ""));
    fakeSearch({ answers: { plan: many }, part: 3 });
    const { user } = renderApp("/w/design/search?q=plan");

    const found = await results();
    await found.findByRole("link", { name: /Roadmap/ });
    expect(found.getAllByRole("listitem")).toHaveLength(3);
    await user.click(found.getByRole("button", { name: "Show more" }));
    await waitFor(() => expect(found.getAllByRole("listitem")).toHaveLength(5));
    expect(found.queryByRole("button", { name: "Show more" })).toBeNull();

    await user.clear(screen.getByRole("searchbox", { name: "Search" }));
    await user.type(screen.getByRole("searchbox", { name: "Search" }), "zebra");
    expect(
      await screen.findByText("Nothing matches. Try fewer words, or other filters."),
    ).toBeInTheDocument();
  });

  it("offers guests no teamspace or person to filter by", async () => {
    fakeWorkspaces({ role: "guest" });
    fakePages();
    fakeSearch();
    renderApp("/w/design/search");

    const filters = within(await screen.findByRole("region", { name: "Filters" }));
    expect(filters.getByRole("combobox", { name: "Kind" })).toBeInTheDocument();
    expect(filters.queryByRole("combobox", { name: "Teamspace" })).toBeNull();
    expect(filters.queryByRole("combobox", { name: "Created by" })).toBeNull();
    // Without a query, what changed last.
    expect(await screen.findByRole("region", { name: "Changed last" })).toBeInTheDocument();
  });
});
