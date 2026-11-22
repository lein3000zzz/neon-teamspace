import { describe, expect, it } from "vitest";
import {
  characters,
  dayProblem,
  displayNameProblem,
  emailProblem,
  endOfDay,
  inAYear,
  MAX_INVITE_LIFETIME_MS,
  inviteCodeProblem,
  inviteUsesProblem,
  noteProblem,
  passwordProblem,
  today,
  tokenNameProblem,
} from "./validation";

describe("the server's rules, checked before sending", () => {
  it("counts characters as the server does, not UTF-16 units", () => {
    expect(characters("é")).toBe(1);
    expect(characters("🙂🙂")).toBe(2);
  });

  it("holds passwords to the minimum in characters and 1024 bytes at most", () => {
    expect(passwordProblem("correct horse", 12)).toBeUndefined();
    expect(passwordProblem("éééééééééééé", 12)).toBeUndefined();
    expect(passwordProblem("ééééééééééé", 12)).toBe("At least 12 characters.");
    expect(passwordProblem("a".repeat(1024), 12)).toBeUndefined();
    expect(passwordProblem("é".repeat(513), 12)).toMatch(/At most 1024 bytes/);
    expect(passwordProblem(" ".repeat(20), 12)).toBe("A password cannot be only spaces.");
  });

  it("takes a bare address", () => {
    expect(emailProblem(" ada@example.com ")).toBeUndefined();
    expect(emailProblem("")).toBe("Fill this in.");
    expect(emailProblem("ada")).toMatch(/name@example.com/);
    expect(emailProblem("Ada <ada@example.com>")).toMatch(/name@example.com/);
    expect(emailProblem(`${"a".repeat(250)}@x.io`)).toMatch(/name@example.com/);
  });

  it("bounds names, notes and token names", () => {
    expect(displayNameProblem("")).toBeUndefined();
    expect(displayNameProblem("a".repeat(100))).toBeUndefined();
    expect(displayNameProblem("a".repeat(101))).toBeDefined();
    expect(displayNameProblem("Ada\nLovelace")).toBeDefined();
    expect(noteProblem("é".repeat(500))).toBeUndefined();
    expect(noteProblem("é".repeat(501))).toBeDefined();
    expect(tokenNameProblem("  ")).toBe("Fill this in.");
    expect(tokenNameProblem("a".repeat(65))).toBeDefined();
  });

  it("takes typed invite codes of 8 to 64 letters, digits, - and _", () => {
    expect(inviteCodeProblem("")).toBeUndefined();
    expect(inviteCodeProblem("Spring-Team_2026")).toBeUndefined();
    expect(inviteCodeProblem("short")).toBeDefined();
    expect(inviteCodeProblem("a".repeat(65))).toBeDefined();
    expect(inviteCodeProblem("with space")).toBeDefined();
    expect(inviteUsesProblem("1")).toBeUndefined();
    expect(inviteUsesProblem("1000")).toBeUndefined();
    expect(inviteUsesProblem("0")).toBeDefined();
    expect(inviteUsesProblem("2.5")).toBeDefined();
  });

  it("takes end dates from today to a year ahead, ending with the day", () => {
    const now = new Date(2026, 10, 21, 15, 0);
    expect(today(now)).toBe("2026-11-21");
    expect(inAYear(now)).toBe("2027-11-20");
    expect(dayProblem("")).toBeUndefined();
    expect(dayProblem("2000-01-01")).toBe("Pick today or a later day.");
    expect(dayProblem("2999-01-01", "2998-12-31")).toBe("Pick a day within a year from today.");
    expect(endOfDay("2026-11-21")).toBe(new Date(2026, 10, 21, 23, 59, 59).toISOString());
  });

  it("ends the last day it offers within the server's 365 days, 29 February or not", () => {
    const cases: [Date, string][] = [
      [new Date(2027, 5, 15, 10, 0), "2028-06-13"],
      [new Date(2028, 1, 1, 9, 0), "2029-01-30"],
      [new Date(2027, 2, 1, 10, 0), "2028-02-28"],
      [new Date(2026, 11, 31, 23, 58), "2027-12-30"],
    ];
    for (const [now, last] of cases) {
      const cap = now.getTime() + MAX_INVITE_LIFETIME_MS;
      expect(inAYear(now)).toBe(last);
      expect(new Date(endOfDay(last)).getTime()).toBeLessThanOrEqual(cap);
    }
  });
});
