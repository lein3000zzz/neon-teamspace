/**
 * The server's rules for what people type, checked before a form is sent so
 * mistakes show at once. The server checks again and has the last word; its
 * messages are shown the same way when it disagrees.
 */
import { strings } from "./strings";

/** The longest password the server takes, in bytes of UTF-8. */
export const MAX_PASSWORD_BYTES = 1024;
/** The longest address the server takes, in bytes. */
export const MAX_EMAIL_LENGTH = 254;
export const MAX_DISPLAY_NAME = 100;
export const MAX_NOTE = 500;
export const MAX_TOKEN_NAME = 64;
export const INVITE_CODE_LENGTH = { min: 8, max: 64 } as const;
export const MAX_INVITE_USES = 1000;

const copy = strings.validation;
const encoder = new TextEncoder();

/** Characters as the server counts them: Unicode scalar values, not UTF-16 units. */
export function characters(text: string): number {
  return [...text].length;
}

function hasControl(text: string): boolean {
  return /\p{Cc}/u.test(text);
}

export function required(value: string): string | undefined {
  return value.trim() === "" ? copy.required : undefined;
}

/** A bare address, name@example.com. The server's parse is stricter; this catches typos. */
export function emailProblem(value: string): string | undefined {
  const address = value.trim();
  if (address === "") {
    return copy.required;
  }
  if (address.length > MAX_EMAIL_LENGTH || !/^[^\s@]+@[^\s@]+$/.test(address)) {
    return copy.email;
  }
  return undefined;
}

/** A new password: the minimum from the server's settings, at most 1024 bytes, not blank. */
export function passwordProblem(value: string, minLength: number): string | undefined {
  if (characters(value) < minLength) {
    return copy.passwordShort(minLength);
  }
  if (encoder.encode(value).length > MAX_PASSWORD_BYTES) {
    return copy.passwordLong;
  }
  if (value.trim() === "") {
    return copy.passwordBlank;
  }
  return undefined;
}

/** Optional: blank takes the address's local part. */
export function displayNameProblem(value: string): string | undefined {
  const name = value.trim();
  return characters(name) > MAX_DISPLAY_NAME || hasControl(name) ? copy.displayName : undefined;
}

export function noteProblem(value: string): string | undefined {
  return characters(value) > MAX_NOTE ? copy.note : undefined;
}

export function tokenNameProblem(value: string): string | undefined {
  const name = value.trim();
  if (name === "") {
    return copy.required;
  }
  return characters(name) > MAX_TOKEN_NAME || hasControl(name) ? copy.tokenName : undefined;
}

/** A code an admin types for an invite; blank lets the server make one. */
export function inviteCodeProblem(value: string): string | undefined {
  const code = value.trim();
  if (code === "") {
    return undefined;
  }
  const { min, max } = INVITE_CODE_LENGTH;
  return code.length < min || code.length > max || !/^[A-Za-z0-9_-]+$/.test(code)
    ? copy.inviteCode
    : undefined;
}

export function inviteUsesProblem(value: string): string | undefined {
  const uses = Number(value);
  return Number.isInteger(uses) && uses >= 1 && uses <= MAX_INVITE_USES ? undefined : copy.maxUses;
}

/** Today in the browser's time zone, as a date input writes it: YYYY-MM-DD. */
export function today(now = new Date()): string {
  return localDate(now);
}

/** The last day an invite may end on: a year from now, less a day for time zones. */
export function inAYear(now = new Date()): string {
  const later = new Date(now);
  later.setFullYear(now.getFullYear() + 1);
  later.setDate(later.getDate() - 1);
  return localDate(later);
}

const pad = (n: number) => String(n).padStart(2, "0");

function localDate(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** The end of a day picked in a date input, in the browser's time zone, as RFC 3339. */
export function endOfDay(day: string): string {
  return new Date(`${day}T23:59:59`).toISOString();
}

/** An optional end date: after today and, when `latest` is given, no later than it. */
export function dayProblem(day: string, latest?: string): string | undefined {
  if (day === "") {
    return undefined;
  }
  if (day < today()) {
    return copy.futureDate;
  }
  if (latest !== undefined && day > latest) {
    return copy.withinYear;
  }
  return undefined;
}
