import { describe, expect, it, onTestFinished } from "bun:test";
import {
  createHash,
  generateKeyPairSync,
  randomBytes,
  sign as ed25519Sign,
  type KeyObject,
} from "node:crypto";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fixture } from "./licenses/fixture.ts";
import {
  PLATFORMS,
  anchor,
  checkSignature,
  readPublicKey,
  updaterFeed,
  type Feed,
  type FeedOptions,
} from "./updater-feed.ts";

const repoRoot = resolve(import.meta.dir, "..");

const CHANGELOG = `# Changelog

## 0.2.0 - 2026-12-01

Apps that update themselves.

## 0.1.0 - 2026-11-28

The first release.
`;

const DATE = new Date("2026-12-01T10:20:30.456Z");

/** A throwaway minisign key pair, the public key in base64 as Tauri's configuration takes it. */
interface Keys {
  id: Buffer;
  privateKey: KeyObject;
  pubkey: string;
}

function makeKeys(): Keys {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const raw = Buffer.from(publicKey.export({ format: "jwk" }).x ?? "", "base64url");
  const id = randomBytes(8);
  const box = `untrusted comment: minisign public key: ${id.toString("hex").toUpperCase()}\n${Buffer.concat([Buffer.from("Ed"), id, raw]).toString("base64")}\n`;
  return { id, privateKey, pubkey: Buffer.from(box).toString("base64") };
}

/**
 * The signature of `file` by `signer`, as Tauri's CLI writes it in a .sig file: a minisign
 * signature of the file's BLAKE2b-512 hash, or with `prehashed` false of the file itself, whose
 * trusted comment names `version`, in base64.
 */
function signature(
  signer: Keys,
  file: Buffer | string,
  version: string | null,
  prehashed = true,
): string {
  const bytes = Buffer.from(file);
  const signed = prehashed ? createHash("blake2b512").update(bytes).digest() : bytes;
  const sig = ed25519Sign(null, signed, signer.privateKey);
  const comment = `timestamp:1764583230\tfile:Neon Teamspace.app.tar.gz${version === null ? "" : `\tversion:${version}`}`;
  const global = ed25519Sign(null, Buffer.concat([sig, Buffer.from(comment)]), signer.privateKey);
  const algorithm = Buffer.from(prehashed ? "ED" : "Ed");
  const text = [
    "untrusted comment: signature from tauri secret key",
    Buffer.concat([algorithm, signer.id, sig]).toString("base64"),
    `trusted comment: ${comment}`,
    global.toString("base64"),
    "",
  ].join("\n");
  return Buffer.from(text).toString("base64");
}

/**
 * A release's files for `tag`: each platform's updater file, and its signature by `signer`
 * naming `version`, unless `files` says otherwise for a name.
 */
function release(
  signer: Keys,
  tag: string,
  version: string,
  files: Record<string, string | null> = {},
): string {
  const written: Record<string, string> = { SHA256SUMS: "" };
  for (const { key, file } of PLATFORMS) {
    const name = file(tag.replace(/^v/, ""));
    const content = `the ${key} update`;
    written[name] = content;
    written[`${name}.sig`] = signature(signer, content, version);
  }
  for (const [name, content] of Object.entries(files)) {
    if (content === null) {
      delete written[name];
    } else {
      written[name] = content;
    }
  }
  return fixture(written, onTestFinished);
}

function options(dir: string, signer: Keys, overrides: Partial<FeedOptions> = {}): FeedOptions {
  return {
    dir,
    tag: "v0.2.0",
    version: "0.2.0",
    changelog: CHANGELOG,
    pubkey: signer.pubkey,
    repository: "lein3000zzz/neon-teamspace",
    date: DATE,
    ...overrides,
  };
}

function feedOf(outcome: ReturnType<typeof updaterFeed>): Feed {
  if (!("feed" in outcome)) {
    throw new Error(`no feed: ${JSON.stringify(outcome)}`);
  }
  return outcome.feed;
}

function problemsOf(outcome: ReturnType<typeof updaterFeed>): string[] {
  if (!("problems" in outcome)) {
    throw new Error(`no problems: ${JSON.stringify(outcome)}`);
  }
  return outcome.problems;
}

describe("updaterFeed", () => {
  it("names each platform's file and signature, the version, the changelog's section and the time", () => {
    const signer = makeKeys();
    const dir = release(signer, "v0.2.0", "0.2.0");

    const feed = feedOf(updaterFeed(options(dir, signer)));

    const download = "https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.2.0";
    const sig = (name: string) => readFileSync(join(dir, `${name}.sig`), "utf8");
    expect(feed).toEqual({
      version: "0.2.0",
      notes:
        "What changed: https://github.com/lein3000zzz/neon-teamspace/blob/v0.2.0/CHANGELOG.md#020---2026-12-01",
      pub_date: "2026-12-01T10:20:30Z",
      platforms: {
        "darwin-aarch64": {
          signature: sig("Neon-Teamspace_0.2.0_aarch64.app.tar.gz"),
          url: `${download}/Neon-Teamspace_0.2.0_aarch64.app.tar.gz`,
        },
        "darwin-x86_64": {
          signature: sig("Neon-Teamspace_0.2.0_x64.app.tar.gz"),
          url: `${download}/Neon-Teamspace_0.2.0_x64.app.tar.gz`,
        },
        "windows-x86_64": {
          signature: sig("Neon-Teamspace_0.2.0_x64-setup.exe"),
          url: `${download}/Neon-Teamspace_0.2.0_x64-setup.exe`,
        },
        "linux-x86_64": {
          signature: sig("Neon-Teamspace_0.2.0_amd64.AppImage"),
          url: `${download}/Neon-Teamspace_0.2.0_amd64.AppImage`,
        },
      },
    });
  });

  it("takes a release candidate's files from its tag, for the version it rehearses", () => {
    const signer = makeKeys();
    const dir = release(signer, "v0.2.0-rc.1", "0.2.0");

    const feed = feedOf(updaterFeed(options(dir, signer, { tag: "v0.2.0-rc.1" })));

    expect(feed.version).toBe("0.2.0");
    expect(feed.notes).toBe(
      "What changed: https://github.com/lein3000zzz/neon-teamspace/blob/v0.2.0-rc.1/CHANGELOG.md#020---2026-12-01",
    );
    expect(feed.platforms["windows-x86_64"]?.url).toBe(
      "https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.2.0-rc.1/Neon-Teamspace_0.2.0-rc.1_x64-setup.exe",
    );
  });

  it("makes nothing of a release built without the signing key", () => {
    const signer = makeKeys();
    const dir = fixture(
      {
        "Neon-Teamspace_0.2.0_x64-setup.exe": "setup",
        "Neon-Teamspace_0.2.0_amd64.AppImage": "appimage",
        "Neon-Teamspace_0.2.0_aarch64.dmg": "dmg",
        SHA256SUMS: "",
      },
      onTestFinished,
    );

    expect(updaterFeed(options(dir, signer))).toEqual({ none: true });
  });

  it("names each platform whose file or signature is missing", () => {
    const signer = makeKeys();
    const dir = release(signer, "v0.2.0", "0.2.0", {
      "Neon-Teamspace_0.2.0_x64-setup.exe.sig": null,
      "Neon-Teamspace_0.2.0_x64.app.tar.gz": null,
    });

    expect(problemsOf(updaterFeed(options(dir, signer)))).toEqual([
      "darwin-x86_64: Neon-Teamspace_0.2.0_x64.app.tar.gz is missing",
      "windows-x86_64: Neon-Teamspace_0.2.0_x64-setup.exe.sig is missing",
    ]);
  });

  it("refuses a signature no app would take", () => {
    const signer = makeKeys();
    const other = makeKeys();
    const dir = release(signer, "v0.2.0", "0.2.0", {
      // Of other bytes, by another key, of an older release, and of no version.
      "Neon-Teamspace_0.2.0_aarch64.app.tar.gz.sig": signature(signer, "other bytes", "0.2.0"),
      "Neon-Teamspace_0.2.0_x64.app.tar.gz.sig": signature(
        other,
        "the darwin-x86_64 update",
        "0.2.0",
      ),
      "Neon-Teamspace_0.2.0_x64-setup.exe.sig": signature(
        signer,
        "the windows-x86_64 update",
        "0.1.0",
      ),
      "Neon-Teamspace_0.2.0_amd64.AppImage.sig": signature(signer, "the linux-x86_64 update", null),
    });

    expect(problemsOf(updaterFeed(options(dir, signer)))).toEqual([
      "darwin-aarch64: Neon-Teamspace_0.2.0_aarch64.app.tar.gz's signature: it does not match the file",
      "darwin-x86_64: Neon-Teamspace_0.2.0_x64.app.tar.gz's signature: it is by another key than the one the apps trust",
      "windows-x86_64: Neon-Teamspace_0.2.0_x64-setup.exe's signature names version 0.1.0, not 0.2.0",
      "linux-x86_64: Neon-Teamspace_0.2.0_amd64.AppImage's signature names no version, not 0.2.0",
    ]);
  });

  it("needs the version's section in the changelog", () => {
    const signer = makeKeys();
    const dir = release(signer, "v0.3.0", "0.3.0");

    expect(
      problemsOf(updaterFeed(options(dir, signer, { tag: "v0.3.0", version: "0.3.0" }))),
    ).toEqual(["CHANGELOG.md has no section for 0.3.0"]);
  });
});

describe("checkSignature", () => {
  it("takes a signature of the file or of its hash, and gives the version it names", () => {
    const signer = makeKeys();
    const file = Buffer.from("an update");

    expect(checkSignature(file, signature(signer, file, "0.2.0"), signer.pubkey)).toBe("0.2.0");
    expect(checkSignature(file, signature(signer, file, "0.2.0", false), signer.pubkey)).toBe(
      "0.2.0",
    );
    expect(checkSignature(file, signature(signer, file, null), signer.pubkey)).toBeNull();
  });

  it("refuses a trusted comment that is not the one signed", () => {
    const signer = makeKeys();
    const file = Buffer.from("an update");
    const text = Buffer.from(signature(signer, file, "0.1.0"), "base64").toString("utf8");
    const forged = Buffer.from(text.replace("version:0.1.0", "version:9.9.9")).toString("base64");

    expect(() => checkSignature(file, forged, signer.pubkey)).toThrow(
      "its trusted comment is not the one signed",
    );
  });

  it("refuses what is not a signature", () => {
    const signer = makeKeys();

    for (const text of [
      "",
      "not base64 at all",
      Buffer.from("untrusted comment: x\n").toString("base64"),
    ]) {
      expect(() => checkSignature(Buffer.from("x"), text, signer.pubkey)).toThrow(
        "it is not a minisign signature",
      );
    }
  });
});

describe("the apps' key and the changelog's anchors", () => {
  it("reads the public key the apps carry", () => {
    const config = JSON.parse(
      readFileSync(join(repoRoot, "apps/shell/src-tauri/tauri.conf.json"), "utf8"),
    ) as { plugins: { updater: { pubkey: string } } };

    const { id, key } = readPublicKey(config.plugins.updater.pubkey);

    expect(id).toHaveLength(8);
    expect(key).toHaveLength(32);
  });

  it("names a section as GitHub does", () => {
    expect(anchor("0.1.0 - 2026-11-28")).toBe("010---2026-11-28");
    expect(anchor("0.2.0 - Unreleased")).toBe("020---unreleased");
    expect(anchor("1.0.0-rc.1 - 2027-01-02")).toBe("100-rc1---2027-01-02");
  });

  it("finds no updater files in a release built without the key, and says so", () => {
    const dir = fixture({ SHA256SUMS: "" }, onTestFinished);

    const run = Bun.spawnSync(
      ["bun", join(import.meta.dir, "updater-feed.ts"), "--dir", dir, "--tag", "v0.2.0"],
      { cwd: repoRoot },
    );

    expect(run.exitCode).toBe(0);
    expect(run.stdout.toString()).toContain("No updater files");
  });
});
