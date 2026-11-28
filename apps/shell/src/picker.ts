import {
  backend,
  displayName,
  failure,
  type Failure,
  type Interrupted,
  type SavedServer,
  type ServerList,
  type Startup,
} from "./backend";

/** The id of the button that stops opening a server. */
const STOP = "stop-opening";

/** What the add form shows while an http address waits for the person to accept it. */
interface Insecure {
  address: string;
  message: string;
}

/** A change to one server under way: its new name, or the question before removing it. */
interface Editing {
  kind: "rename" | "remove";
  origin: string;
}

/** A server the app is opening, which Stop can stop until its page has replaced the picker. */
interface Opening {
  /** Its name, or the address as typed. */
  name: string;
  /** The element focus goes back to once it is stopped, by its id. */
  from: string;
}

interface State {
  list: ServerList;
  /** Whether the add form shows; it always does while the list is empty. */
  adding: boolean;
  editing: Editing | null;
  /** What the app is doing, while a command runs. */
  busy: string | null;
  /** The add form's error. */
  addError: string | null;
  /** A server's error, by its origin. */
  serverErrors: Map<string, string>;
  insecure: Insecure | null;
  /** What the add form's field holds, kept across renders. */
  draft: string;
  opening: Opening | null;
}

/** An element, with attributes, properties set through `on…` handlers, and children. */
function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attributes: Record<string, string | boolean | ((event: Event) => void)> = {},
  ...children: (Node | string | null)[]
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) {
    if (typeof value === "function") {
      element.addEventListener(name.slice(2), value);
    } else if (value === true) {
      element.setAttribute(name, "");
    } else if (value !== false) {
      element.setAttribute(name, value);
    }
  }
  for (const child of children) {
    if (child !== null) {
      element.append(child);
    }
  }
  return element;
}

/**
 * A DOM id for a server, from its origin, one to one: each character but a letter or a digit
 * is written as its code between underscores, so `neon-example.com` and `neon.example.com`
 * get ids of their own.
 */
function idOf(origin: string): string {
  return origin.replace(/[^a-z0-9]/gi, (character) => `_${character.charCodeAt(0).toString(16)}_`);
}

/** The picker: its state, and the page it draws from it. */
export class Picker {
  private state: State = {
    list: { servers: [], last: null },
    adding: false,
    editing: null,
    busy: "Loading your servers…",
    addError: null,
    serverErrors: new Map(),
    insecure: null,
    draft: "",
    opening: null,
  };

  /** The element to focus once the next render is done, by its id. */
  private focusAfter: string | null = null;

  /**
   * The number of the latest attempt to open a server. What a command of an earlier one
   * answers, once it was cancelled or another began, is of no use any more.
   */
  private attempt = 0;

  /** Stops opening a server; shown while one is opening. */
  private readonly cancelButton = h(
    "button",
    { id: STOP, type: "button", class: "button", onclick: () => void this.cancel() },
    "Stop",
  );

  constructor(
    private readonly view: HTMLElement,
    private readonly title: HTMLElement,
    private readonly status: HTMLElement,
    private readonly main: HTMLElement,
    /** Where Stop shows while a server is opening: outside the view, which is inert then. */
    private readonly actions: HTMLElement,
    /** Told what the app said as the picker loads, before the picker does anything with it. */
    private readonly onStartup: (startup: Startup) => void = () => {},
  ) {
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && this.state.opening !== null) {
        void this.cancel();
      }
    });
  }

  async start(): Promise<void> {
    this.render();
    try {
      const startup = await backend.startup();
      this.onStartup(startup);
      this.state.list = { servers: startup.servers, last: startup.last };
      this.state.busy = null;
      if (startup.interrupted !== null) {
        this.interrupted(startup.interrupted);
        return;
      }
      const open = startup.open === null ? undefined : this.server(startup.open);
      if (open !== undefined) {
        await this.open(open);
        return;
      }
      // The first time, the address is all there is to give; after that, the list.
      this.focusAfter = startup.servers.length === 0 ? "address" : "title";
      this.render();
    } catch (error) {
      this.state.busy = null;
      this.state.addError = failure(error).message;
      this.render();
    }
  }

  private server(origin: string): SavedServer | undefined {
    return this.state.list.servers.find((server) => server.origin === origin);
  }

  private set(change: Partial<State>, focus: string | null = null): void {
    Object.assign(this.state, change);
    this.focusAfter = focus;
    this.render();
  }

  /**
   * Runs a command with the page busy, focusing `focus` if given; what it gives back is the
   * new list. The caller ends the busy state.
   */
  private async run(
    busy: string,
    command: () => Promise<ServerList>,
    focus: string | null = null,
  ): Promise<{ ok: true; list: ServerList } | { ok: false; failure: Failure }> {
    this.set({ busy }, focus);
    try {
      const list = await command();
      return { ok: true, list };
    } catch (error) {
      return { ok: false, failure: failure(error) };
    }
  }

  /** Starts an attempt to open a server, which Stop can stop: its number. */
  private begin(opening: Opening): number {
    this.attempt += 1;
    this.state.opening = opening;
    return this.attempt;
  }

  private async open(server: SavedServer): Promise<void> {
    const errors = new Map(this.state.serverErrors);
    errors.delete(server.origin);
    this.state.serverErrors = errors;
    const from = `open-${idOf(server.origin)}`;
    const attempt = this.begin({ name: displayName(server), from });
    const result = await this.run(
      `Opening ${displayName(server)}…`,
      () => backend.openServer(server.origin),
      STOP,
    );
    if (attempt !== this.attempt) {
      return;
    }
    if (result.ok) {
      // The window is loading the server's page; the picker stays as it is, Stop with it,
      // until the page replaces it.
      this.state.list = result.list;
      return;
    }
    errors.set(server.origin, result.failure.message);
    this.set({ busy: null, opening: null, serverErrors: errors }, from);
  }

  private async connect(acceptInsecure: boolean): Promise<void> {
    const address = this.state.insecure?.address ?? this.state.draft;
    this.state.addError = null;
    const typed = address.trim() || "the address";
    const attempt = this.begin({ name: typed, from: "address" });
    const result = await this.run(
      `Checking ${typed}…`,
      () => backend.connect(address, acceptInsecure),
      STOP,
    );
    if (attempt !== this.attempt) {
      return;
    }
    if (result.ok) {
      const opened = result.list.servers.find((server) => server.origin === result.list.last);
      const name = opened === undefined ? "the server" : displayName(opened);
      this.state.list = result.list;
      this.set(
        {
          busy: `Opening ${name}…`,
          insecure: null,
          opening: {
            name,
            from: opened === undefined ? "title" : `open-${idOf(opened.origin)}`,
          },
        },
        STOP,
      );
      return;
    }
    if (result.failure.code === "insecure") {
      this.set(
        { busy: null, opening: null, insecure: { address, message: result.failure.message } },
        "accept-insecure",
      );
      return;
    }
    this.set(
      { busy: null, opening: null, addError: result.failure.message, insecure: null },
      "address",
    );
  }

  /**
   * Stops opening a server. During its check, the picker goes back to where it was; once the
   * window is loading the server's page, the app stops it by loading the picker again, which
   * then says why.
   */
  private async cancel(): Promise<void> {
    const opening = this.state.opening;
    if (opening === null) {
      return;
    }
    // Whatever the command under way answers now is of no use.
    this.attempt += 1;
    this.set({ busy: "Stopping…", opening: null });
    let reloading = false;
    try {
      reloading = await backend.cancelOpening();
    } catch {
      // Only a page that is not the picker is refused: there is nothing left to stop.
    }
    if (reloading) {
      return;
    }
    this.set({ busy: null }, opening.from);
    this.announce(`Stopped opening ${opening.name}.`);
  }

  /** Says why the window came back to the picker from the server it was opening. */
  private interrupted(interrupted: Interrupted): void {
    const server = this.server(interrupted.origin);
    if (interrupted.code === "cancelled") {
      this.set({}, server === undefined ? "title" : `open-${idOf(server.origin)}`);
      this.announce(
        `Stopped opening ${server === undefined ? interrupted.origin : displayName(server)}.`,
      );
      return;
    }
    if (server === undefined) {
      this.set({ addError: interrupted.message }, "address");
      return;
    }
    const errors = new Map(this.state.serverErrors);
    errors.set(server.origin, interrupted.message);
    this.set({ serverErrors: errors }, `open-${idOf(server.origin)}`);
  }

  private async rename(server: SavedServer, label: string): Promise<void> {
    const result = await this.run("Saving the name…", () =>
      backend.renameServer(server.origin, label),
    );
    const errors = new Map(this.state.serverErrors);
    if (result.ok) {
      errors.delete(server.origin);
      const renamed = result.list.servers.find((each) => each.origin === server.origin);
      this.set(
        { list: result.list, editing: null, busy: null, serverErrors: errors },
        `open-${idOf(server.origin)}`,
      );
      this.announce(renamed === undefined ? "" : `Renamed to ${displayName(renamed)}.`);
      return;
    }
    errors.set(server.origin, result.failure.message);
    this.set({ busy: null, serverErrors: errors }, `label-${idOf(server.origin)}`);
  }

  private async remove(server: SavedServer): Promise<void> {
    const at = this.state.list.servers.indexOf(server);
    const result = await this.run(`Removing ${displayName(server)}…`, () =>
      backend.removeServer(server.origin),
    );
    if (result.ok) {
      const next = result.list.servers[Math.min(at, result.list.servers.length - 1)];
      const errors = new Map(this.state.serverErrors);
      errors.delete(server.origin);
      this.set(
        { list: result.list, editing: null, busy: null, serverErrors: errors },
        next === undefined ? "address" : `open-${idOf(next.origin)}`,
      );
      this.announce(`Removed ${displayName(server)}.`);
      return;
    }
    const errors = new Map(this.state.serverErrors);
    errors.set(server.origin, result.failure.message);
    this.set({ busy: null, serverErrors: errors, editing: null }, `open-${idOf(server.origin)}`);
  }

  private announce(message: string): void {
    this.status.textContent = message;
  }

  private render(): void {
    const { busy, list } = this.state;
    const empty = list.servers.length === 0;
    this.main.setAttribute("aria-busy", busy === null ? "false" : "true");
    this.view.inert = busy !== null;
    this.status.textContent = busy ?? "";
    this.title.textContent = empty ? "Connect to a server" : "Choose a server";

    const parts: Node[] = [];
    if (!empty) {
      parts.push(this.renderList());
    }
    if (empty || this.state.adding) {
      parts.push(this.renderAddForm(empty));
    } else {
      parts.push(
        h(
          "button",
          {
            id: "add-server",
            type: "button",
            class: "button",
            onclick: () => this.set({ adding: true, addError: null }, "address"),
          },
          "Add a server",
        ),
      );
    }
    this.view.replaceChildren(...parts);
    // The same button throughout, so that it keeps the focus as the rest is drawn again.
    if (this.state.opening === null) {
      this.cancelButton.remove();
    } else if (!this.cancelButton.isConnected) {
      this.actions.append(this.cancelButton);
    }

    // While the view is inert, only what is outside it can take the focus.
    if (this.focusAfter !== null) {
      const target = document.getElementById(this.focusAfter);
      if (busy === null || (target !== null && !this.view.contains(target))) {
        target?.focus();
        this.focusAfter = null;
      }
    }
  }

  private renderList(): HTMLElement {
    return h(
      "ul",
      { class: "servers", "aria-label": "Saved servers" },
      ...this.state.list.servers.map((server) => this.renderServer(server)),
    );
  }

  private renderServer(server: SavedServer): HTMLElement {
    const id = idOf(server.origin);
    const name = displayName(server);
    const editing = this.state.editing?.origin === server.origin ? this.state.editing : null;
    const error = this.state.serverErrors.get(server.origin);
    const details = [`Neon Teamspace ${server.version}`];
    if (server.label !== undefined && server.label !== server.name) {
      details.push(server.name);
    }
    if (this.state.list.last === server.origin) {
      details.push("last used");
    }

    const open = h(
      "button",
      {
        id: `open-${id}`,
        type: "button",
        class: "server-open",
        "aria-describedby": `details-${id}${error === undefined ? "" : ` error-${id}`}`,
        onclick: () => void this.open(server),
      },
      h("span", { class: "server-name" }, name),
      h("span", { class: "server-origin" }, server.origin),
    );
    // The badge follows the details as one more of them, after a separator: screen readers
    // join the parts' text as it is, and some ran the last detail into the badge without it.
    const meta = h(
      "p",
      { id: `details-${id}`, class: "server-details" },
      details.join(" · "),
      server.encrypted ? null : " · ",
      server.encrypted ? null : h("span", { class: "badge" }, "Not encrypted"),
    );
    const actions = h(
      "div",
      { class: "server-actions" },
      h(
        "button",
        {
          type: "button",
          class: "button quiet",
          "aria-label": `Rename ${name}`,
          "aria-expanded": editing?.kind === "rename" ? "true" : "false",
          onclick: () =>
            this.set({ editing: { kind: "rename", origin: server.origin } }, `label-${id}`),
        },
        "Rename",
      ),
      h(
        "button",
        {
          type: "button",
          class: "button quiet",
          "aria-label": `Remove ${name}`,
          "aria-expanded": editing?.kind === "remove" ? "true" : "false",
          onclick: () =>
            this.set({ editing: { kind: "remove", origin: server.origin } }, `keep-${id}`),
        },
        "Remove",
      ),
    );

    return h(
      "li",
      { class: "server", "data-origin": server.origin },
      open,
      meta,
      actions,
      editing?.kind === "rename" ? this.renderRename(server) : null,
      editing?.kind === "remove" ? this.renderRemove(server) : null,
      error === undefined
        ? null
        : h("p", { id: `error-${id}`, class: "error", role: "alert" }, error),
    );
  }

  private renderRename(server: SavedServer): HTMLElement {
    const id = idOf(server.origin);
    const cancel = () => this.set({ editing: null }, `open-${id}`);
    const input = h("input", {
      id: `label-${id}`,
      name: "label",
      type: "text",
      class: "field",
      maxlength: "80",
      autocomplete: "off",
      placeholder: server.name,
      "aria-describedby": `label-hint-${id}`,
      onkeydown: (event) => {
        if ((event as KeyboardEvent).key === "Escape") {
          cancel();
        }
      },
    });
    input.value = server.label ?? "";
    return h(
      "form",
      {
        class: "inline-form",
        onsubmit: (event) => {
          event.preventDefault();
          void this.rename(server, input.value);
        },
      },
      h("label", { for: `label-${id}` }, `Name for ${server.origin}`),
      input,
      h(
        "p",
        { id: `label-hint-${id}`, class: "hint" },
        `Leave it empty to use the server's own name, ${server.name}.`,
      ),
      h(
        "div",
        { class: "row" },
        h("button", { type: "submit", class: "button primary" }, "Save"),
        h("button", { type: "button", class: "button", onclick: cancel }, "Cancel"),
      ),
    );
  }

  private renderRemove(server: SavedServer): HTMLElement {
    const id = idOf(server.origin);
    const name = displayName(server);
    const keep = () => this.set({ editing: null }, `open-${id}`);
    return h(
      "div",
      {
        class: "inline-form",
        role: "group",
        "aria-labelledby": `remove-question-${id}`,
        onkeydown: (event) => {
          if ((event as KeyboardEvent).key === "Escape") {
            keep();
          }
        },
      },
      h(
        "p",
        { id: `remove-question-${id}` },
        `Remove ${name} from this app? Your account on the server stays, and you can add it again.`,
      ),
      h(
        "div",
        { class: "row" },
        h(
          "button",
          {
            id: `confirm-remove-${id}`,
            type: "button",
            class: "button danger",
            onclick: () => void this.remove(server),
          },
          `Remove ${name}`,
        ),
        h(
          "button",
          { id: `keep-${id}`, type: "button", class: "button", onclick: keep },
          "Keep it",
        ),
      ),
    );
  }

  private renderAddForm(first: boolean): HTMLElement {
    const { addError, insecure } = this.state;
    const describedBy = ["address-hint"];
    if (addError !== null) {
      describedBy.push("address-error");
    }
    const input = h("input", {
      id: "address",
      name: "address",
      type: "text",
      class: "field",
      inputmode: "url",
      autocomplete: "url",
      autocapitalize: "none",
      autocorrect: "off",
      spellcheck: "false",
      placeholder: "neon.example.com",
      enterkeyhint: "go",
      "aria-invalid": addError === null ? "false" : "true",
      "aria-describedby": describedBy.join(" "),
      oninput: (event) => {
        const field = event.target as HTMLInputElement;
        this.state.draft = field.value;
        if (this.state.insecure !== null) {
          // The warning was for the address as it was: editing it takes the warning back,
          // and Connect checks the address as it is now. The caret stays where it was.
          const { selectionStart, selectionEnd } = field;
          this.set({ insecure: null }, "address");
          const again = document.getElementById("address");
          if (again instanceof HTMLInputElement) {
            again.setSelectionRange(selectionStart, selectionEnd);
          }
        }
      },
      onkeydown: (event) => {
        if ((event as KeyboardEvent).key === "Escape" && !first) {
          this.set({ adding: false, addError: null, insecure: null }, "add-server");
        }
      },
    });
    input.value = this.state.draft;

    const buttons = h(
      "div",
      { class: "row" },
      h("button", { type: "submit", class: "button primary" }, "Connect"),
      first
        ? null
        : h(
            "button",
            {
              type: "button",
              class: "button",
              onclick: () =>
                this.set({ adding: false, addError: null, insecure: null }, "add-server"),
            },
            "Cancel",
          ),
    );

    return h(
      "form",
      {
        class: "add",
        "aria-labelledby": first ? "title" : "add-title",
        novalidate: true,
        onsubmit: (event) => {
          event.preventDefault();
          if (this.state.insecure === null) {
            void this.connect(false);
          }
        },
      },
      first ? null : h("h2", { id: "add-title" }, "Add a server"),
      h("label", { for: "address" }, "Server address"),
      input,
      h(
        "p",
        { id: "address-hint", class: "hint" },
        "The address you open Neon Teamspace at in a browser. It starts with https:// unless you type http://.",
      ),
      addError === null
        ? null
        : h("p", { id: "address-error", class: "error", role: "alert" }, addError),
      insecure === null ? buttons : this.renderInsecure(insecure),
    );
  }

  private renderInsecure(insecure: Insecure): HTMLElement {
    return h(
      "div",
      { class: "warning", role: "group", "aria-labelledby": "insecure-title" },
      h("p", { id: "insecure-title", class: "warning-title" }, "This connection is not encrypted"),
      h("p", { role: "alert" }, insecure.message),
      h(
        "div",
        { class: "row" },
        h(
          "button",
          {
            id: "accept-insecure",
            type: "button",
            class: "button danger",
            onclick: () => void this.connect(true),
          },
          "Connect anyway",
        ),
        h(
          "button",
          {
            type: "button",
            class: "button",
            onclick: () => this.set({ insecure: null }, "address"),
          },
          "Cancel",
        ),
      ),
    );
  }
}
