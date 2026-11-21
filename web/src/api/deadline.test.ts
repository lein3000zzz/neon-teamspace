import { describe, expect, it, vi } from "vitest";
import { withDeadline } from "./deadline";

/** A request that only ever settles by being aborted, like a fetch to a stalled server. */
function hang(signal: AbortSignal): Promise<never> {
  return new Promise((_, reject) => {
    signal.addEventListener("abort", () => reject(signal.reason as Error), { once: true });
  });
}

describe("withDeadline", () => {
  it("passes the result through and clears its timer", async () => {
    vi.useFakeTimers();
    await expect(withDeadline(new AbortController().signal, 1000, async () => "ok")).resolves.toBe(
      "ok",
    );
    expect(vi.getTimerCount()).toBe(0);
  });

  it("aborts with a TimeoutError once the deadline passes", async () => {
    vi.useFakeTimers();
    const settled = withDeadline(new AbortController().signal, 1000, hang).catch(
      (error: unknown) => error,
    );
    await vi.advanceTimersByTimeAsync(1000);
    expect(await settled).toMatchObject({ name: "TimeoutError" });
  });

  it("aborts with the caller's reason when the caller cancels first", async () => {
    vi.useFakeTimers();
    const caller = new AbortController();
    const result = withDeadline(caller.signal, 1000, hang);
    caller.abort(new DOMException("cancelled", "AbortError"));
    await expect(result).rejects.toMatchObject({ name: "AbortError" });
    expect(vi.getTimerCount()).toBe(0);
  });

  it("starts aborted when the caller already cancelled", async () => {
    const caller = new AbortController();
    caller.abort();
    await expect(withDeadline(caller.signal, 1000, async (signal) => signal.aborted)).resolves.toBe(
      true,
    );
  });
});
