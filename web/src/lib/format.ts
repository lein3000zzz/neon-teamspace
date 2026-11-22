import { strings } from "./strings";

const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
const date = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });

/** A moment from the API, RFC 3339, in the reader's locale and time zone. */
export function formatDateTime(iso: string): string {
  return dateTime.format(new Date(iso));
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
