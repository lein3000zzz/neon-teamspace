/**
 * Runs a request with a deadline. The signal handed to `request` aborts when
 * `signal` does (TanStack Query cancelling) or once `ms` have passed, with the
 * same TimeoutError that AbortSignal.timeout uses. A server that accepts the
 * connection and never answers then fails like any other unreachable server.
 *
 * AbortSignal.any would combine the two signals, but Safari only has it from
 * 17.4 and the build targets older versions.
 */
export async function withDeadline<T>(
  signal: AbortSignal,
  ms: number,
  request: (signal: AbortSignal) => Promise<T>,
): Promise<T> {
  const controller = new AbortController();
  const forward = () => controller.abort(signal.reason);
  if (signal.aborted) {
    forward();
  } else {
    signal.addEventListener("abort", forward, { once: true });
  }
  const timer = setTimeout(() => {
    controller.abort(new DOMException(`No answer within ${ms} ms`, "TimeoutError"));
  }, ms);
  try {
    return await request(controller.signal);
  } finally {
    clearTimeout(timer);
    signal.removeEventListener("abort", forward);
  }
}
