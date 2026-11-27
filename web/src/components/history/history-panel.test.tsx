import { screen, waitFor, within } from "@testing-library/react";
import { beforeAll, describe, expect, it } from "vitest";
import { fakeHistory, VERSION_IDS } from "@/test/history";
import { renderApp } from "@/test/render-app";
import { fakePages, page, PAGE_IDS, samplePages } from "@/test/pages";
import { fakeWorkspaces, TEAMSPACE_IDS } from "@/test/workspaces";

const PATH = `/w/design/p/${PAGE_IDS.roadmap}`;

async function openHistory(user: ReturnType<typeof renderApp>["user"]) {
  await user.click(await screen.findByRole("button", { name: "History" }));
  return within(await screen.findByRole("dialog", { name: "Page history" }));
}

/** The classes of the glyph before a block's text in a change. */
function glyphOf(text: HTMLElement | undefined) {
  return text?.parentElement?.querySelector("svg")?.getAttribute("class");
}

describe("a page's history", () => {
  // The preview loads the editor on demand; loaded once here, so no test waits on it.
  beforeAll(async () => {
    await import("@/components/editor/version-view");
  });

  it("lists the versions with when, who and how much, and what changed since the latest", async () => {
    fakeWorkspaces();
    fakePages();
    fakeHistory();
    const { user } = renderApp(PATH);

    const panel = await openHistory(user);
    const versions = within(panel.getByRole("region", { name: "Versions" }));
    const latest = await versions.findByRole("button", { name: /1 added, 1 changed/ });
    expect(latest).toHaveAttribute("aria-pressed", "true");
    expect(within(latest).getByText("Bob")).toBeInTheDocument();
    expect(versions.getByRole("button", { name: /2 added/ })).toHaveAttribute(
      "aria-pressed",
      "false",
    );

    const changes = within(
      await panel.findByRole("list", { name: "Changes from this version to now" }),
    );
    expect(
      panel.getByText("Since this version: 2 added, 3 changed, 1 removed."),
    ).toBeInTheDocument();
    expect(changes.getByText("Kick-off in March").tagName).toBe("DEL");
    expect(changes.getByText("Kick-off in April").tagName).toBe("INS");
    // A level deeper than the blocks around it.
    expect(changes.getByText("Hire a designer").closest("li")?.style.marginLeft).toBe("1.25rem");
    expect(changes.getByText("Old idea").tagName).toBe("DEL");

    // Blocks read as the page shows them, not as markdown: a heading's text, a list's
    // items after their bullets and numbers, which stay out of what is struck through.
    expect(changes.getByText("Roadmap")).toBeInTheDocument();
    expect(changes.queryByText(/#/)).toBeNull();
    const demo = changes.getByText("A demo");
    expect(demo.tagName).toBe("DEL");
    expect(demo.previousElementSibling?.textContent).toBe("•");
    expect(changes.getByText("Rehearse").previousElementSibling?.textContent).toBe("2.");
    // A file by its name and caption, without its path.
    expect(
      changes.getByText("checklist.pdf Print two", {
        normalizer: (text) => text.replace(/\s+/g, " "),
      }),
    ).toBeInTheDocument();
    expect(changes.queryByText(/api\/v1\/files/)).toBeNull();
    // A to-do item ticked since: its box empty as it was, ticked as it is, and said so.
    const [then, now] = changes.getAllByText("Book the room");
    expect(then?.tagName).toBe("DEL");
    expect(then?.previousElementSibling?.textContent).toBe("To do:");
    expect(now?.tagName).toBe("INS");
    expect(now?.previousElementSibling?.textContent).toBe("Done:");
    expect(glyphOf(then)).toMatch(/\blucide-square\b(?!-)/);
    expect(glyphOf(now)).toContain("lucide-square-check");
  });

  it("shows a version as it was, read-only", async () => {
    fakeWorkspaces();
    fakePages();
    fakeHistory();
    const { user } = renderApp(PATH);

    const panel = await openHistory(user);
    await user.click(await panel.findByRole("button", { name: /2 added/ }));
    // The tabs: one picked and reached by Tab, the arrow keys moving between them.
    const tabs = within(panel.getByRole("tablist", { name: "Show" }));
    const changes = tabs.getByRole("tab", { name: "Changes since" });
    const previewTab = tabs.getByRole("tab", { name: "Preview" });
    expect(changes).toHaveAttribute("aria-selected", "true");
    expect(changes).toHaveAttribute("tabindex", "0");
    expect(previewTab).toHaveAttribute("tabindex", "-1");
    expect(panel.getByRole("tabpanel", { name: "Changes since" }).id).toBe(
      changes.getAttribute("aria-controls"),
    );
    changes.focus();
    await user.keyboard("{ArrowRight}");
    expect(previewTab).toHaveFocus();
    expect(previewTab).toHaveAttribute("aria-selected", "true");
    expect(panel.getByRole("tabpanel", { name: "Preview" })).toBeInTheDocument();
    await user.keyboard("{Home}");
    expect(changes).toHaveFocus();
    await user.keyboard("{ArrowLeft}");
    expect(previewTab).toHaveFocus();
    const preview = await panel.findByRole("region", { name: "This version, read-only" });
    await waitFor(() => expect(preview).toHaveTextContent("Kick-off in March"));
    expect(within(preview).getByRole("textbox")).toHaveAttribute("contenteditable", "false");
  });

  it("restores a version after asking, on the revision it was compared at", async () => {
    fakeWorkspaces();
    fakePages();
    const api = fakeHistory();
    const { user } = renderApp(PATH);

    const panel = await openHistory(user);
    await user.click(await panel.findByRole("button", { name: /2 added/ }));
    await panel.findByRole("list", { name: "Changes from this version to now" });
    await user.click(panel.getByRole("button", { name: "Restore this version" }));
    const confirm = within(await screen.findByRole("alertdialog"));
    expect(confirm.getByText(/kept in its history first/)).toBeInTheDocument();
    await user.click(confirm.getByRole("button", { name: "Cancel" }));
    expect(api.restores).toEqual([]);

    await user.click(panel.getByRole("button", { name: "Restore this version" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Restore" }),
    );
    expect(
      await panel.findByText("Restored. The page shows this version again."),
    ).toBeInTheDocument();
    expect(api.restores).toEqual([{ version: VERSION_IDS.first, body: { base: "rev-1" } }]);
  });

  it("refuses a restore once the page changed, and compares again to try again", async () => {
    fakeWorkspaces();
    fakePages();
    const api = fakeHistory({ changedTo: "rev-2" });
    const { user } = renderApp(PATH);

    const panel = await openHistory(user);
    await panel.findByRole("list", { name: "Changes from this version to now" });
    const asked = api.compared();
    await user.click(panel.getByRole("button", { name: "Restore this version" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Restore" }),
    );
    expect(await panel.findByText(/The page changed while you were looking/)).toBeInTheDocument();
    await waitFor(() => expect(api.compared()).toBeGreaterThan(asked));

    await user.click(panel.getByRole("button", { name: "Restore this version" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Restore" }),
    );
    expect(
      await panel.findByText("Restored. The page shows this version again."),
    ).toBeInTheDocument();
    // The restore is listed above, and the version restored stays picked.
    expect(await panel.findByRole("button", { name: /Restored from/ })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(panel.getByRole("button", { name: /1 added, 1 changed/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(api.restores.map((restore) => restore.body)).toEqual([
      { base: "rev-1" },
      { base: "rev-2" },
    ]);
  });

  it("offers no history to those who only view the page", async () => {
    fakeWorkspaces();
    fakePages({
      pages: [
        page(
          {
            id: PAGE_IDS.roadmap,
            title: "Roadmap",
            teamspace: TEAMSPACE_IDS.general,
            access: "view",
          },
          "a0",
        ),
        ...samplePages().filter((entry) => entry.id !== PAGE_IDS.roadmap),
      ],
    });
    const api = fakeHistory();
    renderApp(PATH);

    expect(await screen.findByRole("heading", { level: 1, name: "Roadmap" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "History" })).toBeNull();
    expect(api.compared()).toBe(0);
  });

  it("restores nothing while the page is as the version has it", async () => {
    fakeWorkspaces();
    fakePages();
    fakeHistory({ unchanged: true });
    const { user } = renderApp(PATH);

    const panel = await openHistory(user);
    expect(await panel.findByText("The page is as this version has it.")).toBeInTheDocument();
    expect(panel.getByRole("button", { name: "Restore this version" })).toBeDisabled();
  });

  it("says when a page has no version yet", async () => {
    fakeWorkspaces();
    fakePages();
    fakeHistory({ versions: [] });
    const { user } = renderApp(PATH);

    const panel = await openHistory(user);
    expect(await panel.findByText(/No versions yet/)).toBeInTheDocument();
  });
});
