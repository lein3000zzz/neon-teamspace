import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderApp } from "@/test/render-app";
import { fakePages, PAGE_IDS, samplePage } from "@/test/pages";
import { fakeSearch, result } from "@/test/search";
import { fakeWorkspaces } from "@/test/workspaces";

const roadmap = samplePage(PAGE_IDS.roadmap);
const launch = samplePage(PAGE_IDS.launch);

async function dialog() {
  return within(await screen.findByRole("dialog", { name: "Search" }));
}

describe("the quick search", () => {
  it("opens on Ctrl+K with the pages changed last, and finds what is typed", async () => {
    fakeWorkspaces();
    fakePages();
    const api = fakeSearch({
      answers: {
        road: [result(roadmap, "Road", "the quarterly roadmap"), result(launch, "", "on the road")],
      },
    });
    const { user } = renderApp("/w/design");
    await screen.findByRole("heading", { level: 1, name: "Design" });

    await user.keyboard("{Control>}k{/Control}");
    const search = await dialog();
    const recent = within(await search.findByRole("region", { name: "Recently changed" }));
    expect(await recent.findByRole("button", { name: /Roadmap/ })).toBeInTheDocument();
    expect(recent.getByRole("button", { name: /Notes/ })).toBeInTheDocument();
    expect(search.getByRole("searchbox", { name: "Search pages" })).toHaveFocus();

    await user.keyboard("road");
    const results = within(await search.findByRole("region", { name: "Results" }));
    const first = await results.findByRole("button", { name: /Roadmap/ });
    expect(within(first).getByText("Road").tagName).toBe("MARK");
    expect(results.getByText("on the road")).toBeInTheDocument();
    expect(
      results.getByRole("button", { name: "Search everything for “road”" }),
    ).toBeInTheDocument();
    expect(api.asked.at(-1)).toEqual({ query: "road", limit: "8" });
  });

  it("moves through the results with the arrow keys and opens one with Enter", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSearch({ answers: { road: [result(roadmap, "Road", ""), result(launch, "", "")] } });
    const { user, router } = renderApp("/w/design");
    await screen.findByRole("heading", { level: 1, name: "Design" });

    await user.keyboard("{Meta>}k{/Meta}");
    const search = await dialog();
    await user.keyboard("road");
    const results = within(await search.findByRole("region", { name: "Results" }));
    await results.findByRole("button", { name: /Launch/ });

    await user.keyboard("{ArrowDown}");
    expect(results.getByRole("button", { name: /Roadmap/ })).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(results.getByRole("button", { name: /Launch/ })).toHaveFocus();
    await user.keyboard("{ArrowUp}{ArrowUp}");
    expect(search.getByRole("searchbox", { name: "Search pages" })).toHaveFocus();
    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");

    await waitFor(() =>
      expect(router.state.location.pathname).toBe(`/w/design/p/${PAGE_IDS.launch}`),
    );
    expect(screen.queryByRole("dialog", { name: "Search" })).toBeNull();
  });

  it("opens the first result with Enter in the field, and the search page from its last line", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSearch({ answers: { road: [result(roadmap, "Road", "")] } });
    const { user, router } = renderApp("/w/design");
    await screen.findByRole("heading", { level: 1, name: "Design" });

    await user.keyboard("{Control>}k{/Control}");
    let search = await dialog();
    await user.keyboard("road");
    await within(await search.findByRole("region", { name: "Results" })).findByRole("button", {
      name: /Roadmap/,
    });
    await user.keyboard("{Enter}");
    await waitFor(() =>
      expect(router.state.location.pathname).toBe(`/w/design/p/${PAGE_IDS.roadmap}`),
    );

    // From the sidebar's button, then to the search page with what was typed.
    await user.click(
      within(screen.getByRole("complementary")).getByRole("button", { name: /Search/ }),
    );
    search = await dialog();
    await user.keyboard("road");
    await user.click(await search.findByRole("button", { name: "Search everything for “road”" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/w/design/search"));
    expect(router.state.location.search).toEqual({ q: "road" });
  });

  it("says when nothing matches, and closes with Escape", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSearch();
    const { user } = renderApp("/w/design");
    await screen.findByRole("heading", { level: 1, name: "Design" });

    await user.keyboard("{Control>}k{/Control}");
    const search = await dialog();
    await user.keyboard("zebra");
    expect(await search.findByText("Nothing matches.")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Search" })).toBeNull());
  });
});
