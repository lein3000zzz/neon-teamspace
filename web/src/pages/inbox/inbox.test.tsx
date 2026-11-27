import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { afterInboxClose, inboxFeedServer } from "@/api/inbox-feed";
import { COMMENT_IDS } from "@/test/comments";
import { fakeInbox, notification, NOTIFICATION_IDS } from "@/test/inbox";
import { fakePages, PAGE_IDS, samplePages } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { asPerson, fakeWorkspaces, people, scout, studioInvitation } from "@/test/workspaces";

const roadmap = samplePages().find((each) => each.id === PAGE_IDS.roadmap);
const launch = samplePages().find((each) => each.id === PAGE_IDS.launch);

/** Bob mentioned Ada in a comment on Roadmap, and assigned her Launch. */
function sample() {
  return [
    notification({
      id: NOTIFICATION_IDS.first,
      kind: "mention",
      actor: people.bob,
      ...(roadmap ? { page: roadmap } : {}),
      thread: COMMENT_IDS.thread,
      comment: COMMENT_IDS.first,
      excerpt: "Can you look at the dates?",
    }),
    notification({
      id: NOTIFICATION_IDS.second,
      kind: "assigned",
      actor: people.bob,
      ...(launch ? { page: launch } : {}),
    }),
  ];
}

/** A notification in the list, by what its link says. */
async function item(name: RegExp) {
  const found = (await screen.findByRole("link", { name })).closest("li");
  if (found === null) {
    throw new Error("no notification");
  }
  return within(found);
}

describe("the inbox", () => {
  it("lists what is unread, each a link to the thread it is about that marks it read", async () => {
    fakeWorkspaces();
    fakePages();
    const api = fakeInbox({ notifications: sample() });
    const { user, router } = renderApp("/inbox");

    const mention = await item(/Bob mentioned you in a comment on Roadmap/);
    expect(mention.getByText("Can you look at the dates?")).toBeInTheDocument();
    expect(mention.getByText("Unread")).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: "Bob assigned you to Launch" })).toHaveAttribute(
      "href",
      `/w/design/p/${PAGE_IDS.launch}`,
    );
    await user.click(mention.getByRole("link", { name: /Bob mentioned you/ }));

    await waitFor(() =>
      expect(router.state.location.pathname).toBe(`/w/design/p/${PAGE_IDS.roadmap}`),
    );
    expect(router.state.location.search).toEqual({ comment: COMMENT_IDS.thread });
    expect(api.changes).toEqual([
      {
        method: "PATCH",
        path: `/api/v1/notifications/${NOTIFICATION_IDS.first}`,
        body: { read: true },
      },
    ]);
  });

  it("names an agent with whom it acts for, as mail does", async () => {
    fakeWorkspaces();
    fakePages();
    fakeInbox({
      notifications: [
        notification({
          id: NOTIFICATION_IDS.first,
          kind: "reply",
          actor: asPerson(scout),
          ...(roadmap ? { page: roadmap } : {}),
          thread: COMMENT_IDS.thread,
          comment: COMMENT_IDS.reply,
          excerpt: "The chart card is not started.",
        }),
      ],
    });
    renderApp("/inbox");

    expect(
      await screen.findByRole("link", { name: "Scout via Ada replied to your comment on Roadmap" }),
    ).toBeInTheDocument();
  });

  it("marks one unread again, archives one, and shows each where it went", async () => {
    fakeWorkspaces();
    fakePages();
    fakeInbox({ notifications: sample() });
    const { user } = renderApp("/inbox");

    const assigned = await item(/Bob assigned you to Launch/);
    await user.click(
      assigned.getByRole("button", { name: "Mark read: Bob assigned you to Launch" }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("link", { name: "Bob assigned you to Launch" })).toBeNull(),
    );
    await user.click(screen.getByRole("radio", { name: "All" }));
    const read = await item(/Bob assigned you to Launch/);
    expect(read.queryByText("Unread")).toBeNull();
    await user.click(read.getByRole("button", { name: "Archive: Bob assigned you to Launch" }));
    await waitFor(() =>
      expect(screen.queryByRole("link", { name: "Bob assigned you to Launch" })).toBeNull(),
    );

    await user.click(screen.getByRole("radio", { name: "Archived" }));
    const archived = await item(/Bob assigned you to Launch/);
    await user.click(
      archived.getByRole("button", { name: "Back to the inbox: Bob assigned you to Launch" }),
    );
    expect(await screen.findByText("Nothing is archived.")).toBeInTheDocument();
  });

  it("marks everything read at once, and the sidebar stops counting", async () => {
    fakeWorkspaces();
    fakePages();
    const api = fakeInbox({ notifications: sample() });
    const { user } = renderApp("/inbox");
    const sidebar = within(await screen.findByRole("complementary"));
    expect(await sidebar.findByRole("link", { name: "Inbox, 2 unread" })).toBeInTheDocument();

    await user.click(await screen.findByRole("button", { name: "Mark all read" }));

    expect(await screen.findByText("Nothing unread.")).toBeInTheDocument();
    expect(await sidebar.findByRole("link", { name: "Inbox" })).toBeInTheDocument();
    expect(api.changes.map((change) => change.path)).toEqual(["/api/v1/inbox/read"]);
  });

  it("shows what comes to it as it comes, from its live feed", async () => {
    fakeWorkspaces();
    fakePages();
    const api = fakeInbox();
    renderApp("/inbox");
    expect(await screen.findByText("Nothing unread.")).toBeInTheDocument();

    api.state.notifications = sample();
    api.tell();

    expect(
      await screen.findByRole("link", { name: "Bob assigned you to Launch" }),
    ).toBeInTheDocument();
    const sidebar = within(screen.getByRole("complementary"));
    expect(await sidebar.findByRole("link", { name: "Inbox, 2 unread" })).toBeInTheDocument();
  });

  it("shows the invitations waiting above, to answer there", async () => {
    fakeWorkspaces({ received: [studioInvitation] });
    fakePages();
    fakeInbox();
    renderApp("/inbox");

    const invitations = within(await screen.findByRole("region", { name: "Invitations" }));
    expect(await invitations.findByRole("button", { name: "Accept: Studio" })).toBeInTheDocument();
  });

  it("names the feed's address from the page's, and what to do as it closes", () => {
    expect(inboxFeedServer({ protocol: "https:", host: "neon.example.com" })).toBe(
      "wss://neon.example.com/ws/inbox",
    );
    expect(inboxFeedServer({ protocol: "http:", host: "127.0.0.1:3000" })).toBe(
      "ws://127.0.0.1:3000/ws/inbox",
    );
    expect(afterInboxClose(4401)).toBe("signedOut");
    expect(afterInboxClose(4408)).toBe("later");
    expect(afterInboxClose(1013)).toBe("later");
    expect(afterInboxClose(1012)).toBe("again");
  });
});
