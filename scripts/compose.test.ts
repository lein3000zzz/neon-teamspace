import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const repoRoot = resolve(import.meta.dir, "..");

interface Service {
  networks?: string[] | Record<string, unknown>;
  ports?: unknown[];
}

interface Compose {
  services: Record<string, Service>;
  networks?: Record<string, { internal?: boolean } | null>;
}

const compose = Bun.YAML.parse(
  readFileSync(join(repoRoot, "deployments/docker-compose.yml"), "utf8"),
) as Compose;

/** The service `name` of the Compose file. */
function service(name: string): Service {
  const found = compose.services[name];
  if (found === undefined) throw new Error(`docker-compose.yml has no ${name} service`);
  return found;
}

/** The networks Compose puts `of` on: the stack's default one unless it names others. */
function networks(of: Service): string[] {
  const named = of.networks;
  if (named === undefined) return ["default"];
  return Array.isArray(named) ? named : Object.keys(named);
}

describe("docker-compose.yml", () => {
  it("lets the server alone reach the updater, which holds the Docker socket", () => {
    const updater = service("updater");
    expect(networks(updater)).toEqual(["updater"]);
    expect(updater.ports).toBeUndefined();
    expect(compose.networks?.updater?.internal).toBe(true);

    const on = Object.entries(compose.services)
      .filter(([, each]) => networks(each).includes("updater"))
      .map(([name]) => name)
      .toSorted();
    expect(on).toEqual(["server", "updater"]);
    expect(networks(service("server"))).toContain("default");
  });
});
