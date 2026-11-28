/**
 * Writes `latest.json`, the feed of the desktop apps' updater, for a release: the release's
 * version, a link to its section of the changelog as its notes, the time, and for each platform
 * the file the updater installs, as the release attaches it, with its signature. The apps read
 * the feed of the latest release, at releases/latest/download/latest.json, and install a file
 * only when its signature holds.
 *
 * The release's desktop builds make those files when they have the updater's signing key:
 * Tauri's CLI signs each one, in a `.sig` file beside it. Each signature is checked here, as
 * the apps check it: by the key whose public half the apps carry, in tauri.conf.json, over the
 * file as attached, naming the apps' version, which the apps require. A release whose files no
 * app would take fails here, rather than in every app.
 *
 * A release built without the key, as a fork's is, has none of the files: nothing is written,
 * and the output says so. One with only some of them fails.
 *
 * Usage: bun scripts/updater-feed.ts --dir <the release's files> --tag vX.Y.Z[-rc.N]
 *        [--repository owner/name]
 */

import { createHash, createPublicKey, verify } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

/** The repository the apps' releases come from, as the apps' updater names it. */
export const REPOSITORY = "lein3000zzz/neon-teamspace";

/** Each platform's key in the feed, and the name of its updater file in a release `release`. */
export const PLATFORMS: { key: string; file: (release: string) => string }[] = [
  { key: "darwin-aarch64", file: (release) => `Neon-Teamspace_${release}_aarch64.app.tar.gz` },
  { key: "darwin-x86_64", file: (release) => `Neon-Teamspace_${release}_x64.app.tar.gz` },
  { key: "windows-x86_64", file: (release) => `Neon-Teamspace_${release}_x64-setup.exe` },
  { key: "linux-x86_64", file: (release) => `Neon-Teamspace_${release}_amd64.AppImage` },
];

/** What `latest.json` holds, as Tauri's updater reads it. */
export interface Feed {
  version: string;
  notes: string;
  pub_date: string;
  platforms: Record<string, { signature: string; url: string }>;
}

/** What a release's files make. */
export type Outcome = { feed: Feed } | { problems: string[] } | { none: true };

export interface FeedOptions {
  /** The directory of the release's files. */
  dir: string;
  /** The release's tag, such as v0.2.0 or v0.2.0-rc.1, which its files' names follow. */
  tag: string;
  /** The apps' version, which their signatures must name: the one the release rehearses. */
  version: string;
  /** The text of CHANGELOG.md, whose section of `version` the notes link. */
  changelog: string;
  /** The updater's public key, as tauri.conf.json has it. */
  pubkey: string;
  repository: string;
  date: Date;
}

/** The DER prefix of an Ed25519 public key, before its 32 bytes. */
const ED25519_SPKI = Buffer.from("302a300506032b6570032100", "hex");

/** A changelog's section heading: `## 0.2.0 - 2026-12-01`, or `## 0.2.0 - Unreleased`. */
const SECTION = /^## (\S+) - .+$/gm;

/** The anchor GitHub gives a heading: lower case, without punctuation, hyphens for spaces. */
export function anchor(heading: string): string {
  return heading
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{M}\p{N} _-]/gu, "")
    .replace(/ /g, "-");
}

/** The heading of `version`'s section of the changelog, without its `## `. */
function section(changelog: string, version: string): string | undefined {
  for (const match of changelog.matchAll(SECTION)) {
    if (match[1] === version) {
      return match[0].slice(3);
    }
  }
  return undefined;
}

/** The lines of a base64 text, as minisign writes its keys and signatures. */
function lines(base64: string): string[] {
  return Buffer.from(base64.trim(), "base64").toString("utf8").split(/\r?\n/);
}

/** The bytes of a base64 line, or none when it is not base64. */
function decoded(line: string | undefined): Buffer | undefined {
  if (line === undefined || !/^[A-Za-z0-9+/]+={0,2}$/.test(line)) {
    return undefined;
  }
  return Buffer.from(line, "base64");
}

/** An Ed25519 public key, from its 32 bytes. */
function ed25519(raw: Buffer) {
  return createPublicKey({ key: Buffer.concat([ED25519_SPKI, raw]), format: "der", type: "spki" });
}

/** The key id and the Ed25519 key of a minisign public key in base64, as Tauri takes it. */
export function readPublicKey(pubkey: string): { id: Buffer; key: Buffer } {
  const bytes = decoded(lines(pubkey)[1]?.trim());
  if (bytes?.length !== 42 || bytes.subarray(0, 2).toString("latin1") !== "Ed") {
    throw new Error("the updater's public key is not a minisign public key");
  }
  return { id: bytes.subarray(2, 10), key: bytes.subarray(10) };
}

/**
 * Checks a signature, a minisign signature file in base64 as Tauri's CLI writes it, of `file`
 * by `pubkey`: the signature of the file, or of its BLAKE2b-512 hash, and the global signature
 * of that signature and its trusted comment. The version the trusted comment names, if any.
 */
export function checkSignature(file: Buffer, signature: string, pubkey: string): string | null {
  const { id, key } = readPublicKey(pubkey);
  const [, sigLine, commentLine, globalLine] = lines(signature).map((line) => line.trim());
  const sig = decoded(sigLine);
  const global = decoded(globalLine);
  if (
    sig?.length !== 74 ||
    global?.length !== 64 ||
    commentLine?.startsWith("trusted comment: ") !== true
  ) {
    throw new Error("it is not a minisign signature");
  }
  const algorithm = sig.subarray(0, 2).toString("latin1");
  if (algorithm !== "ED" && algorithm !== "Ed") {
    throw new Error(`its algorithm ${JSON.stringify(algorithm)} is not minisign's`);
  }
  if (!sig.subarray(2, 10).equals(id)) {
    throw new Error("it is by another key than the one the apps trust");
  }
  const signed = algorithm === "ED" ? createHash("blake2b512").update(file).digest() : file;
  const publicKey = ed25519(key);
  if (!verify(null, signed, publicKey, sig.subarray(10))) {
    throw new Error("it does not match the file");
  }
  const comment = commentLine.slice("trusted comment: ".length);
  if (!verify(null, Buffer.concat([sig.subarray(10), Buffer.from(comment)]), publicKey, global)) {
    throw new Error("its trusted comment is not the one signed");
  }
  const version = comment.split("\t").find((field) => field.startsWith("version:"));
  return version === undefined ? null : version.slice("version:".length);
}

/** The feed of the release whose files are in `options.dir`, or why there is none. */
export function updaterFeed(options: FeedOptions): Outcome {
  const release = options.tag.replace(/^v/, "");
  const found = PLATFORMS.map(({ key, file }) => {
    const name = file(release);
    const path = join(options.dir, name);
    return { key, name, path, file: existsSync(path), sig: existsSync(`${path}.sig`) };
  });
  const macArchives = found.filter((each) => each.key.startsWith("darwin") && each.file);
  if (found.every((each) => !each.sig) && macArchives.length === 0) {
    return { none: true };
  }

  const problems: string[] = [];
  const heading = section(options.changelog, options.version);
  if (heading === undefined) {
    problems.push(`CHANGELOG.md has no section for ${options.version}`);
  }
  try {
    readPublicKey(options.pubkey);
  } catch (error) {
    return { problems: [(error as Error).message] };
  }
  const platforms: Feed["platforms"] = {};
  for (const each of found) {
    if (!each.file || !each.sig) {
      problems.push(`${each.key}: ${each.file ? `${each.name}.sig` : each.name} is missing`);
      continue;
    }
    const signature = readFileSync(`${each.path}.sig`, "utf8").trim();
    try {
      const signed = checkSignature(readFileSync(each.path), signature, options.pubkey);
      if (signed !== options.version) {
        problems.push(
          `${each.key}: ${each.name}'s signature names ${signed === null ? "no version" : `version ${signed}`}, not ${options.version}`,
        );
        continue;
      }
    } catch (error) {
      problems.push(`${each.key}: ${each.name}'s signature: ${(error as Error).message}`);
      continue;
    }
    platforms[each.key] = {
      signature,
      url: `https://github.com/${options.repository}/releases/download/${options.tag}/${encodeURIComponent(each.name)}`,
    };
  }
  if (problems.length > 0 || heading === undefined) {
    return { problems };
  }
  return {
    feed: {
      version: options.version,
      notes: `What changed: https://github.com/${options.repository}/blob/${options.tag}/CHANGELOG.md#${anchor(heading)}`,
      pub_date: options.date.toISOString().replace(/\.\d{3}Z$/, "Z"),
      platforms,
    },
  };
}

/** The apps' version, from their crate's manifest. */
function appVersion(root: string): string {
  const manifest = Bun.TOML.parse(
    readFileSync(join(root, "apps/shell/src-tauri/Cargo.toml"), "utf8"),
  ) as { package?: { version?: unknown } };
  const version = manifest.package?.version;
  if (typeof version !== "string") {
    throw new Error("apps/shell/src-tauri/Cargo.toml has no version");
  }
  return version;
}

/** The updater's public key, from the apps' configuration. */
function appPubkey(root: string): string {
  const config = JSON.parse(
    readFileSync(join(root, "apps/shell/src-tauri/tauri.conf.json"), "utf8"),
  ) as { plugins?: { updater?: { pubkey?: unknown } } };
  const pubkey = config.plugins?.updater?.pubkey;
  if (typeof pubkey !== "string") {
    throw new Error("apps/shell/src-tauri/tauri.conf.json has no updater public key");
  }
  return pubkey;
}

function main(args: readonly string[]): number {
  const usage =
    "Usage: bun scripts/updater-feed.ts --dir <the release's files> --tag vX.Y.Z[-rc.N] [--repository owner/name]";
  const options = new Map<string, string>();
  for (let index = 0; index < args.length; index += 2) {
    const [name, value] = [args[index], args[index + 1]];
    if (!["--dir", "--tag", "--repository"].includes(name ?? "") || value === undefined) {
      console.error(usage);
      return 2;
    }
    options.set(name as string, value);
  }
  const dir = options.get("--dir");
  const tag = options.get("--tag");
  if (dir === undefined || tag === undefined) {
    console.error(usage);
    return 2;
  }
  const root = resolve(import.meta.dir, "..");
  const outcome = updaterFeed({
    dir,
    tag,
    version: appVersion(root),
    changelog: readFileSync(join(root, "CHANGELOG.md"), "utf8"),
    pubkey: appPubkey(root),
    repository: options.get("--repository") ?? REPOSITORY,
    date: new Date(),
  });
  if ("none" in outcome) {
    console.log(
      `No updater files in ${dir}: the desktop builds had no signing key, so the release has no latest.json.`,
    );
    return 0;
  }
  if ("problems" in outcome) {
    console.error("The updater files cannot make latest.json:");
    for (const problem of outcome.problems) {
      console.error(`  ${problem}`);
    }
    return 1;
  }
  const out = join(dir, "latest.json");
  writeFileSync(out, `${JSON.stringify(outcome.feed, null, 2)}\n`);
  console.log(
    `Wrote ${out} for ${outcome.feed.version}: ${Object.keys(outcome.feed.platforms).join(", ")}.`,
  );
  return 0;
}

if (import.meta.main) {
  process.exit(main(process.argv.slice(2)));
}
