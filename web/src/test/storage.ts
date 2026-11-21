import { vi } from "vitest";

/**
 * Makes reading window.localStorage throw, as browsers do when the user
 * blocks site data. The spy is restored after each test.
 */
export function blockStorage() {
  vi.spyOn(window, "localStorage", "get").mockImplementation(() => {
    throw new DOMException("The operation is insecure.", "SecurityError");
  });
}

/** Makes every write to localStorage fail as it does when the quota is used up. */
export function fillStorage() {
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new DOMException("The quota has been exceeded.", "QuotaExceededError");
  });
}
