import { backend, failure } from "./backend";

/**
 * The picker's check for updates, which only the Android app shows: the desktop apps check
 * from their menu, and the iOS app does not update itself. The app's Rust side asks GitHub for
 * the latest release; a newer one is offered in a dialog of the app's own, which downloads its
 * APK in the browser. The picker says what the check found.
 */
export class Updates {
  private readonly button = document.createElement("button");
  private readonly status = document.createElement("p");
  private readonly error = document.createElement("p");
  private checking = false;

  constructor(private readonly container: HTMLElement) {
    this.button.type = "button";
    this.button.id = "check-for-updates";
    this.button.className = "button quiet";
    this.button.textContent = "Check for updates";
    this.button.addEventListener("click", () => void this.check());
    this.status.id = "update-status";
    this.status.className = "update-status";
    this.status.setAttribute("role", "status");
    this.error.id = "update-error";
    this.error.className = "error";
    this.error.setAttribute("role", "alert");
    this.button.setAttribute("aria-describedby", "update-status update-error");
  }

  /** Shows the control, once the app has said it offers the check. */
  show(): void {
    this.container.replaceChildren(this.button, this.status, this.error);
    this.container.hidden = false;
  }

  private async check(): Promise<void> {
    // A second press while the first is under way asks nothing more.
    if (this.checking) {
      return;
    }
    this.checking = true;
    this.button.setAttribute("aria-disabled", "true");
    this.error.textContent = "";
    this.status.textContent = "Checking for updates…";
    try {
      const found = await backend.checkForUpdates();
      this.status.textContent =
        found.state === "current"
          ? `Neon Teamspace ${found.version} is the latest version.`
          : `Neon Teamspace ${found.version} is available. You have ${found.current}.`;
    } catch (error) {
      this.status.textContent = "";
      this.error.textContent = failure(error).message;
    } finally {
      this.checking = false;
      this.button.removeAttribute("aria-disabled");
    }
  }
}
