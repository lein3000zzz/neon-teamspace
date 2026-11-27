import { strings } from "./strings";

const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
const date = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });
const dateTimeThisYear = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

/** A moment from the API, RFC 3339, in the reader's locale and time zone. */
export function formatDateTime(iso: string): string {
  return dateTime.format(new Date(iso));
}

/**
 * A moment as {@link formatDateTime} has it, without its year when that is
 * the year of `now`: Nov 27, 12:30 AM. For where room is short.
 */
export function formatShortDateTime(iso: string, now: Date = new Date()): string {
  const moment = new Date(iso);
  return moment.getFullYear() === now.getFullYear()
    ? dateTimeThisYear.format(moment)
    : dateTime.format(moment);
}

export function formatDate(iso: string): string {
  return date.format(new Date(iso));
}

/** How long to wait, from a Retry-After in seconds, in the largest unit that reads well. */
export function formatWait(seconds: number): string {
  const copy = strings.wait;
  if (seconds < 60) {
    return copy.seconds(Math.max(1, Math.ceil(seconds)));
  }
  if (seconds < 3600) {
    return copy.minutes(Math.ceil(seconds / 60));
  }
  return copy.hours(Math.ceil(seconds / 3600));
}

/**
 * A browser and system named from a User-Agent header, such as "Firefox on
 * Linux", or undefined when nothing in it is recognised. Only for telling
 * sessions apart, so a rough guess is enough.
 */
export function describeUserAgent(agent: string): string | undefined {
  const browser = BROWSERS.find(([pattern]) => pattern.test(agent))?.[1];
  const system = SYSTEMS.find(([pattern]) => pattern.test(agent))?.[1];
  if (browser !== undefined && system !== undefined) {
    return strings.account.sessions.browserOn(browser, system);
  }
  return browser ?? system;
}

// Order matters: Edge and Opera say Chrome too, and Chrome says Safari.
const BROWSERS: readonly [RegExp, string][] = [
  [/Edg\//, "Edge"],
  [/OPR\//, "Opera"],
  [/Firefox\//, "Firefox"],
  [/Chrome\//, "Chrome"],
  [/Safari\//, "Safari"],
  [/^curl\//, "curl"],
];

// iOS and Android before the desktop systems their strings also mention.
const SYSTEMS: readonly [RegExp, string][] = [
  [/iPhone|iPad|iPod/, "iOS"],
  [/Android/, "Android"],
  [/Windows/, "Windows"],
  [/Mac OS X|Macintosh/, "macOS"],
  [/CrOS/, "ChromeOS"],
  [/Linux/, "Linux"],
];

const BYTE_UNITS = ["B", "KiB", "MiB", "GiB", "TiB"] as const;
const decimal = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

/**
 * A size in the largest binary unit, 1024 apart, that keeps it at 1 or more:
 * 1.5 GiB, 0 B. The server's limits and messages name sizes so, as do the
 * admins' limit fields.
 */
export function formatBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${decimal.format(value)} ${BYTE_UNITS[unit]}`;
}
