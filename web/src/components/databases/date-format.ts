import { useSyncExternalStore } from "react";
import { DATE_FORMATS, type DateFormat } from "@/lib/database-rules";
import { browserStorage } from "@/theme/preferences";

const keyFor = (propertyId: string) => `neon.date-format.${propertyId}`;

/** Components that show the same property hear each other's choice; other tabs, through storage. */
const listeners = new Set<() => void>();

/** What was chosen while the page is open, which holds where storage is refused. */
const chosen = new Map<string, DateFormat>();

function stored(propertyId: string): DateFormat | undefined {
  try {
    const value = browserStorage()?.getItem(keyFor(propertyId));
    return DATE_FORMATS.find((format) => format === value);
  } catch {
    return undefined;
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  window.addEventListener("storage", listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", listener);
  };
}

/**
 * Keeps the format a date property's days show in, which each reader picks for
 * themselves, in this browser: the server keeps the dates, not how they read.
 */
export function rememberDateFormat(propertyId: string, format: DateFormat) {
  chosen.set(propertyId, format);
  try {
    browserStorage()?.setItem(keyFor(propertyId), format);
  } catch {
    // Kept for as long as the page is open, above.
  }
  for (const listener of listeners) {
    listener();
  }
}

/** The format the reader picked for a date property's days, medium unless they picked one. */
export function useDateFormat(propertyId: string): DateFormat {
  return useSyncExternalStore(
    subscribe,
    () => stored(propertyId) ?? chosen.get(propertyId) ?? "medium",
  );
}
