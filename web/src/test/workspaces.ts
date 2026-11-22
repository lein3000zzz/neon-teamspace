import { http, HttpResponse, type HttpResponseResolver } from "msw";
import type { Schemas } from "@/api/client";
import type { Teamspace, TeamspaceMember, TeamspaceName } from "@/api/teamspaces";
import type {
  InviteLink,
  Member,
  Person,
  ReceivedInvitation,
  SentInvitation,
  Workspace,
  WorkspaceRole,
  WorkspaceSummary,
} from "@/api/workspaces";
import { hueFor } from "@/lib/hue";
import { ada, apiError } from "./fixtures";
import { server } from "./server";

/** People in the workspaces of the tests, Ada, who is signed in by default, first. */
export const people = {
  ada: { id: ada.id, display_name: "Ada", email: "ada@example.com", hue: ada.hue },
  bob: {
    id: "0193a5b2-0000-7000-8000-00000000c001",
    display_name: "Bob",
    email: "bob@example.com",
    hue: 177,
  },
  cy: {
    id: "0193a5b2-0000-7000-8000-00000000c002",
    display_name: "Cy",
    email: "cy@example.com",
    hue: 0,
  },
  gus: {
    id: "0193a5b2-0000-7000-8000-00000000c003",
    display_name: "Gus",
    email: "gus@example.com",
    hue: 179,
  },
} as const satisfies Record<string, Person>;

const WORKSPACE_ID = "0193a5b2-0000-7000-8000-00000000a001";
const OTHER_ID = "0193a5b2-0000-7000-8000-00000000a002";
export const TEAMSPACE_IDS = {
  general: "0193a5b2-0000-7000-8000-00000000b001",
  open: "0193a5b2-0000-7000-8000-00000000b002",
  closed: "0193a5b2-0000-7000-8000-00000000b003",
  private: "0193a5b2-0000-7000-8000-00000000b004",
} as const;

export const LINK_TOKEN = "wsl_4yVd0q3oJ2pLmN8sTuVwXyZaBcDeFgHiJkLmNoPqRsT";

const CREATED = "2026-11-20T09:00:00Z";

const noContent = () => new HttpResponse(null, { status: 204 });

/**
 * Design, which Ada owns unless a test gives her another role, with Bob as its
 * admin, or its owner when she is not, and Cy and Gus as member and guest.
 */
export function design(role: WorkspaceRole = "owner"): Workspace {
  return {
    id: WORKSPACE_ID,
    name: "Design",
    slug: "design",
    icon: "🚀",
    hue: hueFor(WORKSPACE_ID),
    role,
    created_at: CREATED,
    updated_at: CREATED,
    members_can_create_teamspaces: true,
    member_count: 4,
    limits: {},
  };
}

/** A second workspace, for the switcher. */
export const studio: WorkspaceSummary = {
  id: OTHER_ID,
  name: "Studio",
  slug: "studio",
  hue: hueFor(OTHER_ID),
  role: "member",
  created_at: CREATED,
};

function teamspace(
  key: keyof typeof TEAMSPACE_IDS,
  name: string,
  access: Teamspace["access"],
  role: Teamspace["role"],
  count: number,
): Teamspace {
  const id = TEAMSPACE_IDS[key];
  return {
    id,
    workspace_id: WORKSPACE_ID,
    name,
    hue: hueFor(id),
    access,
    member_count: count,
    created_at: CREATED,
    updated_at: CREATED,
    ...(role === undefined ? {} : { role }),
  };
}

export const teamspaces = {
  general: teamspace("general", "General", "default", "owner", 3),
  open: teamspace("open", "Open studio", "open", undefined, 1),
  closed: teamspace("closed", "Closed room", "closed", undefined, 1),
  private: teamspace("private", "Secret lab", "private", "owner", 1),
};

const member = (person: Person, role: WorkspaceRole): Member => ({
  user: person,
  role,
  joined_at: CREATED,
});

const inTeamspace = (person: Person, role: Teamspace["role"]): TeamspaceMember => ({
  user: person,
  role: role ?? "member",
  joined_at: CREATED,
});

type Options = {
  /** Ada's role in Design. */
  role?: WorkspaceRole;
  canCreate?: boolean;
  /** The workspaces Ada is in; Design alone unless given. */
  workspaces?: WorkspaceSummary[];
  teamspaces?: Teamspace[];
  hidden?: TeamspaceName[];
  invitations?: SentInvitation[];
  links?: InviteLink[];
  received?: ReceivedInvitation[];
};

/** A request a test may look at: what was asked, and with what body. */
export type Recorded = { method: string; path: string; body: unknown };

/**
 * The workspace routes of the server, for Design and Ada in it, holding what
 * changes so pages read back what they did. Every request that changes
 * something is recorded. A test overrides one route with server.use() after
 * this, for an error, say.
 */
export function fakeWorkspaces(options: Options = {}) {
  const role = options.role ?? "owner";
  const state = {
    workspace: design(role),
    workspaces: options.workspaces ?? [summaryOf(design(role))],
    canCreate: options.canCreate ?? true,
    // Bob owns the workspace whenever Ada does not, so it always has an owner.
    members: [
      member(people.ada, role),
      member(people.bob, role === "owner" ? "admin" : "owner"),
      member(people.cy, "member"),
      member(people.gus, "guest"),
    ],
    teamspaces: options.teamspaces ?? [teamspaces.general, teamspaces.open, teamspaces.closed],
    hidden: options.hidden ?? [],
    teamspaceMembers: {
      [TEAMSPACE_IDS.general]: [
        inTeamspace(people.ada, "owner"),
        inTeamspace(people.bob, "member"),
        inTeamspace(people.cy, "member"),
      ],
      [TEAMSPACE_IDS.open]: [inTeamspace(people.bob, "owner")],
      [TEAMSPACE_IDS.closed]: [inTeamspace(people.bob, "owner")],
      [TEAMSPACE_IDS.private]: [inTeamspace(people.ada, "owner")],
    } as Record<string, TeamspaceMember[]>,
    invitations: options.invitations ?? [],
    links: options.links ?? [],
    received: options.received ?? [],
  };
  const changes: Recorded[] = [];

  const record = (
    resolver: (body: unknown, params: Record<string, string>) => Response | Promise<Response>,
  ) =>
    (async ({ request, params }) => {
      const text = await request.text();
      const body: unknown = text === "" ? undefined : JSON.parse(text);
      changes.push({ method: request.method, path: new URL(request.url).pathname, body });
      return resolver(body, params as Record<string, string>);
    }) satisfies HttpResponseResolver;

  const base = "/api/v1/workspaces/:id";
  const visible = (id: string) => state.teamspaces.find((entry) => entry.id === id);

  server.use(
    http.get("/api/v1/workspaces", () =>
      HttpResponse.json({ workspaces: state.workspaces, can_create: state.canCreate }),
    ),
    http.post(
      "/api/v1/workspaces",
      record((body) => {
        const input = body as Schemas["CreateWorkspace"];
        const created: Workspace = {
          ...design("owner"),
          id: "0193a5b2-0000-7000-8000-00000000a009",
          name: input.name,
          slug: input.slug ?? input.name.toLowerCase().replaceAll(" ", "-"),
          member_count: 1,
          ...(input.icon ? { icon: input.icon } : { icon: null }),
        };
        state.workspaces = [...state.workspaces, summaryOf(created)];
        return HttpResponse.json(created, { status: 201 });
      }),
    ),
    http.get(base, ({ params }) =>
      params["id"] === state.workspace.id
        ? HttpResponse.json(state.workspace)
        : apiError(404, "not_found", "no such workspace"),
    ),
    http.patch(
      base,
      record((body) => {
        const patch = body as Schemas["UpdateWorkspace"];
        const next: Workspace = { ...state.workspace };
        if (patch.name) next.name = patch.name;
        if (patch.slug) next.slug = patch.slug;
        if (patch.icon !== undefined) next.icon = patch.icon;
        if (patch.hue_override !== undefined) {
          next.hue_override = patch.hue_override;
          next.hue = patch.hue_override ?? hueFor(next.id);
        }
        if (typeof patch.members_can_create_teamspaces === "boolean") {
          next.members_can_create_teamspaces = patch.members_can_create_teamspaces;
        }
        state.workspace = next;
        state.workspaces = state.workspaces.map((entry) =>
          entry.id === next.id ? summaryOf(next) : entry,
        );
        return HttpResponse.json(next);
      }),
    ),
    http.delete(
      base,
      record((body) => {
        if ((body as Schemas["DeleteWorkspace"]).confirm_name !== state.workspace.name) {
          return apiError(400, "confirmation_mismatch", "the name typed is not the workspace's");
        }
        state.workspaces = state.workspaces.filter((entry) => entry.id !== state.workspace.id);
        return noContent();
      }),
    ),
    http.get(`${base}/members`, () => HttpResponse.json({ members: state.members })),
    http.patch(
      `${base}/members/:user_id`,
      record((body, params) => {
        const target = state.members.find((entry) => entry.user.id === params["user_id"]);
        if (target === undefined) {
          return apiError(404, "not_found", "no such member");
        }
        target.role = (body as Schemas["ChangeRole"]).role;
        return HttpResponse.json(target);
      }),
    ),
    http.delete(
      `${base}/members/:user_id`,
      record((_, params) => {
        state.members = state.members.filter((entry) => entry.user.id !== params["user_id"]);
        return noContent();
      }),
    ),
    http.post(
      `${base}/leave`,
      record(() => {
        state.workspaces = state.workspaces.filter((entry) => entry.id !== state.workspace.id);
        return noContent();
      }),
    ),
    http.post(
      `${base}/transfer`,
      record((body) => {
        const target = state.members.find(
          (entry) => entry.user.id === (body as Schemas["TransferOwnership"]).user_id,
        );
        if (target) target.role = "owner";
        const self = state.members.find((entry) => entry.user.id === people.ada.id);
        if (self) self.role = "admin";
        state.workspace = { ...state.workspace, role: "admin" };
        state.workspaces = state.workspaces.map((entry) =>
          entry.id === state.workspace.id ? { ...entry, role: "admin" } : entry,
        );
        return noContent();
      }),
    ),
    http.get(`${base}/invitations`, () => HttpResponse.json({ invitations: state.invitations })),
    http.post(
      `${base}/invitations`,
      record((body) => {
        const input = body as Schemas["CreateInvitation"];
        const invitee =
          Object.values(people).find((person) => person.email === input.email) ??
          ({
            id: "0193a5b2-0000-7000-8000-00000000c009",
            display_name: "Dee",
            email: input.email,
            hue: 10,
          } satisfies Person);
        const sent: SentInvitation = {
          id: "0193a5b2-0000-7000-8000-00000000d009",
          invitee,
          role: input.role,
          created_at: CREATED,
          expires_at: "2026-12-04T09:00:00Z",
        };
        state.invitations = [sent, ...state.invitations];
        return HttpResponse.json(sent, { status: 201 });
      }),
    ),
    http.post(
      `${base}/invitations/:invitation_id/revoke`,
      record((_, params) => {
        state.invitations = state.invitations.filter(
          (entry) => entry.id !== params["invitation_id"],
        );
        return noContent();
      }),
    ),
    http.get(`${base}/invite-links`, () => HttpResponse.json({ links: state.links })),
    http.post(
      `${base}/invite-links`,
      record((body) => {
        const input = body as Schemas["CreateInviteLink"];
        const link: InviteLink = {
          id: "0193a5b2-0000-7000-8000-00000000e009",
          role: input.role,
          max_uses: input.max_uses ?? 10,
          uses: 0,
          status: "open",
          created_at: CREATED,
          expires_at: input.expires_at ?? "2026-11-27T09:00:00Z",
          token_kept: false,
        };
        state.links = [link, ...state.links];
        return HttpResponse.json({ link, token: LINK_TOKEN }, { status: 201 });
      }),
    ),
    http.get(`${base}/invite-links/:link_id/token`, () => HttpResponse.json({ token: LINK_TOKEN })),
    http.post(
      `${base}/invite-links/:link_id/revoke`,
      record((_, params) => {
        state.links = state.links.map((entry) =>
          entry.id === params["link_id"] ? { ...entry, status: "revoked" } : entry,
        );
        return noContent();
      }),
    ),
    http.get(`${base}/teamspaces`, () =>
      role === "guest"
        ? HttpResponse.json({ teamspaces: [], hidden: [] })
        : HttpResponse.json({ teamspaces: state.teamspaces, hidden: state.hidden }),
    ),
    http.post(
      `${base}/teamspaces`,
      record((body) => {
        const input = body as Schemas["CreateTeamspace"];
        const created: Teamspace = {
          ...teamspaces.open,
          id: "0193a5b2-0000-7000-8000-00000000b009",
          name: input.name,
          access: input.access,
          role: "owner",
          member_count: 1,
          ...(input.icon ? { icon: input.icon } : {}),
        };
        state.teamspaces = [...state.teamspaces, created];
        state.teamspaceMembers[created.id] = [inTeamspace(people.ada, "owner")];
        return HttpResponse.json(created, { status: 201 });
      }),
    ),
    http.get(`${base}/teamspaces/:teamspace_id`, ({ params }) => {
      const found = visible(String(params["teamspace_id"]));
      return found && (found.access !== "private" || found.role)
        ? HttpResponse.json(found)
        : apiError(404, "not_found", "no such teamspace");
    }),
    http.patch(
      `${base}/teamspaces/:teamspace_id`,
      record((body, params) => {
        const patch = body as Schemas["UpdateTeamspace"];
        const id = params["teamspace_id"] ?? "";
        const found = visible(id);
        const hidden = state.hidden.find((entry) => entry.id === id);
        if (hidden && patch.name) {
          hidden.name = patch.name;
        }
        const next: Teamspace = {
          ...(found ?? { ...teamspaces.private, id, name: hidden?.name ?? "", role: null }),
          ...(patch.name ? { name: patch.name } : {}),
          ...(patch.access ? { access: patch.access } : {}),
          ...(patch.icon === undefined ? {} : { icon: patch.icon }),
          ...(patch.hue_override === undefined ? {} : { hue_override: patch.hue_override }),
        };
        if (found) {
          state.teamspaces = state.teamspaces.map((entry) => (entry.id === id ? next : entry));
        }
        return HttpResponse.json(next);
      }),
    ),
    http.delete(
      `${base}/teamspaces/:teamspace_id`,
      record((_, params) => {
        state.teamspaces = state.teamspaces.filter((entry) => entry.id !== params["teamspace_id"]);
        state.hidden = state.hidden.filter((entry) => entry.id !== params["teamspace_id"]);
        return noContent();
      }),
    ),
    http.post(
      `${base}/teamspaces/:teamspace_id/join`,
      record((_, params) => {
        const id = String(params["teamspace_id"]);
        state.teamspaces = state.teamspaces.map((entry) =>
          entry.id === id
            ? { ...entry, role: "member", member_count: entry.member_count + 1 }
            : entry,
        );
        state.teamspaceMembers[id] = [
          ...(state.teamspaceMembers[id] ?? []),
          inTeamspace(people.ada, "member"),
        ];
        return HttpResponse.json(visible(id));
      }),
    ),
    http.post(
      `${base}/teamspaces/:teamspace_id/leave`,
      record((_, params) => {
        const id = String(params["teamspace_id"]);
        state.teamspaces = state.teamspaces.map((entry) =>
          entry.id === id ? { ...entry, role: null, member_count: entry.member_count - 1 } : entry,
        );
        state.teamspaceMembers[id] = (state.teamspaceMembers[id] ?? []).filter(
          (entry) => entry.user.id !== people.ada.id,
        );
        return noContent();
      }),
    ),
    http.get(`${base}/teamspaces/:teamspace_id/members`, ({ params }) =>
      HttpResponse.json({ members: state.teamspaceMembers[String(params["teamspace_id"])] ?? [] }),
    ),
    http.post(
      `${base}/teamspaces/:teamspace_id/members`,
      record((body, params) => {
        const input = body as Schemas["AddTeamspaceMember"];
        const person = Object.values(people).find((entry) => entry.id === input.user_id);
        if (person === undefined) {
          return apiError(404, "not_found", "no such member");
        }
        const added = inTeamspace(person, input.role ?? "member");
        const id = String(params["teamspace_id"]);
        state.teamspaceMembers[id] = [...(state.teamspaceMembers[id] ?? []), added];
        return HttpResponse.json(added, { status: 201 });
      }),
    ),
    http.patch(
      `${base}/teamspaces/:teamspace_id/members/:user_id`,
      record((body, params) => {
        const list = state.teamspaceMembers[String(params["teamspace_id"])] ?? [];
        const target = list.find((entry) => entry.user.id === params["user_id"]);
        if (target === undefined) {
          return apiError(404, "not_found", "no such member");
        }
        target.role = (body as Schemas["ChangeTeamspaceRole"]).role;
        return HttpResponse.json(target);
      }),
    ),
    http.delete(
      `${base}/teamspaces/:teamspace_id/members/:user_id`,
      record((_, params) => {
        const id = String(params["teamspace_id"]);
        state.teamspaceMembers[id] = (state.teamspaceMembers[id] ?? []).filter(
          (entry) => entry.user.id !== params["user_id"],
        );
        return noContent();
      }),
    ),
    http.get("/api/v1/account/invitations", () =>
      HttpResponse.json({ invitations: state.received }),
    ),
    http.post(
      "/api/v1/account/invitations/:id/accept",
      record((_, params) => {
        const invitation = state.received.find((entry) => entry.id === params["id"]);
        if (invitation === undefined) {
          return apiError(404, "not_found", "no such invitation");
        }
        state.received = state.received.filter((entry) => entry !== invitation);
        const joined: WorkspaceSummary = {
          ...invitation.workspace,
          role: invitation.role,
          created_at: CREATED,
        };
        state.workspaces = [...state.workspaces, joined];
        return HttpResponse.json(joined);
      }),
    ),
    http.post(
      "/api/v1/account/invitations/:id/decline",
      record((_, params) => {
        state.received = state.received.filter((entry) => entry.id !== params["id"]);
        return noContent();
      }),
    ),
  );
  return { state, changes };
}

/** A workspace as the list has it. */
export function summaryOf(workspace: Workspace): WorkspaceSummary {
  const {
    members_can_create_teamspaces: _members,
    member_count: _count,
    limits: _limits,
    updated_at: _updated,
    ...summary
  } = workspace;
  return summary;
}

/** An invitation into Studio, waiting for Ada. */
export const studioInvitation: ReceivedInvitation = {
  id: "0193a5b2-0000-7000-8000-00000000f001",
  workspace: { id: studio.id, name: studio.name, slug: studio.slug, hue: studio.hue },
  role: "member",
  invited_by: people.bob,
  created_at: CREATED,
  expires_at: "2026-12-04T09:00:00Z",
};
