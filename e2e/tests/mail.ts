/**
 * Mailpit's API, which make e2e passes in E2E_MAIL_URL; empty against a stack started another
 * way, where the tests that read mail are skipped.
 */
export const MAIL = process.env.E2E_MAIL_URL ?? "";
