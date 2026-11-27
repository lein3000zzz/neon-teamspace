/**
 * The e2e stack's mail, which its Mailpit catches: the tests wait for a mail as it comes and
 * follow its links, as the person it is for would.
 */
import { expect, type APIRequestContext } from "@playwright/test";
import type { Meta } from "./fixtures";

/**
 * Mailpit's API, which make e2e passes in E2E_MAIL_URL; empty against a stack started another
 * way, where the tests that read mail are skipped.
 */
export const MAIL = process.env.E2E_MAIL_URL ?? "";

/** The instance's name, which the subjects of its mail carry. */
export async function instanceName(request: APIRequestContext): Promise<string> {
  const response = await request.get("/api/v1/meta");
  expect(response.status(), "GET /api/v1/meta").toBe(200);
  return ((await response.json()) as Meta).name;
}

/** A mail as Mailpit lists it. */
type Summary = { ID: string; To: { Address: string }[]; Subject: string };

/** A mail Mailpit caught: its subject and its plain text part. */
export type Mail = { subject: string; text: string };

/** The mails to `to` with a subject `subject` matches, newest first. */
async function mailsTo(to: string, subject: string | RegExp): Promise<Summary[]> {
  const query = new URLSearchParams({ query: `to:"${to}"` });
  const response = await fetch(`${MAIL}/api/v1/search?${query.toString()}`);
  expect(response.status, "Mailpit's search").toBe(200);
  const { messages } = (await response.json()) as { messages: Summary[] };
  return messages.filter(
    (message) =>
      message.To.some((recipient) => recipient.Address === to) &&
      (typeof subject === "string" ? message.Subject === subject : subject.test(message.Subject)),
  );
}

/**
 * The mail to `to` with a subject `subject` matches, once it has come. The server sends mail
 * from its queue, a moment after what it is about, so this waits for exactly one.
 */
export async function mailTo(to: string, subject: string | RegExp): Promise<Mail> {
  let found: Summary[] = [];
  await expect
    .poll(
      async () => {
        found = await mailsTo(to, subject);
        return found.length;
      },
      { message: `a mail to ${to}: ${String(subject)}`, timeout: 30_000 },
    )
    .toBe(1);
  const response = await fetch(`${MAIL}/api/v1/message/${found[0]?.ID ?? ""}`);
  expect(response.status, "Mailpit's message").toBe(200);
  const message = (await response.json()) as { Subject: string; Text: string };
  return { subject: message.Subject, text: message.Text };
}

/**
 * The address of the link a mail's text gives on a line of its own, after `label`: the
 * stack's web origin, as the server's public URL is.
 */
export function linkIn(mail: Mail, label: string, origin: string): string {
  const line = mail.text.split(/\r?\n/).find((text) => text.startsWith(`${label}: `));
  expect(line, `"${label}" in the mail "${mail.subject}"`).toBeDefined();
  const url = (line ?? "").slice(label.length + 2).trim();
  expect(new URL(url).origin, `where "${label}" leads`).toBe(origin);
  return url;
}
