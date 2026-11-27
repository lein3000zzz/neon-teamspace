/**
 * The people a workspace's content names, agents and integrations among them,
 * and how each is named: a person by their name, an agent or integration with
 * the person it acts for, "Scout · via Ada Lovelace", the server's own words.
 */
import type { Agent } from "@/api/agents";
import type { Member, Person } from "@/api/workspaces";
import { strings } from "@/lib/strings";

/** Someone content names: a member, or an agent or integration deleted since. */
export type Someone = Person & { deleted?: boolean };

/** An agent or an integration: a member that acts for a person, its owner. */
export function actsForOwner(person: Pick<Person, "kind">): boolean {
  return person.kind !== "human";
}

/**
 * Whether person properties may name them, to give them work: people and
 * agents; never integrations, which only deliver what their webhook's sender
 * posts; nor anyone deleted.
 */
export function assignable(person: Someone): boolean {
  return person.kind !== "integration" && person.deleted !== true;
}

/** Someone by name, with whom they act for put as `via` puts it. */
function named(person: Someone, via: (name: string, owner: string) => string): string {
  if (!actsForOwner(person)) {
    return person.display_name;
  }
  const owner = person.owner?.display_name;
  const both = owner === undefined ? person.display_name : via(person.display_name, owner);
  return person.deleted === true ? strings.people.deleted(both) : both;
}

/** How someone is named on one line, as exports and the server name them. */
export function attribution(person: Someone): string {
  return named(person, strings.people.via);
}

/**
 * How someone is named inside a sentence, as notification mail names them:
 * "Scout via Ada replied to your comment".
 */
export function attributionInSentence(person: Someone): string {
  return named(person, strings.people.viaInSentence);
}

/** An agent or integration as content names it, deleted or not. */
function fromAgent(agent: Agent): Someone {
  return {
    id: agent.id,
    kind: agent.kind,
    display_name: agent.name,
    hue: agent.hue,
    ...(agent.owner ? { owner: agent.owner } : {}),
    ...(agent.status === "deleted" ? { deleted: true } : {}),
  };
}

/**
 * Everyone a workspace's content may name, by id: its members, then its
 * agents and integrations no longer in it, which stay named as deleted.
 */
export function peopleById(
  members: readonly Member[],
  agents: readonly Agent[],
): Map<string, Someone> {
  const people = new Map<string, Someone>(agents.map((agent) => [agent.id, fromAgent(agent)]));
  for (const member of members) {
    people.set(member.user.id, member.user);
  }
  return people;
}
