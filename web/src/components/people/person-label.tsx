import { cn } from "cn";
import { Bot, Webhook } from "lucide-react";
import { HueDot } from "@/components/nodes/hue-dot";
import { actsForOwner, type Someone } from "@/lib/people";
import { strings } from "@/lib/strings";

const copy = strings.people;

/** The small mark of an agent or an integration beside its name. */
export function KindBadge({ kind }: { kind: Someone["kind"] }) {
  if (kind === "human") {
    return null;
  }
  const Icon = kind === "agent" ? Bot : Webhook;
  return (
    <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full border border-border px-1.5 text-[0.6875rem] leading-4 text-muted-foreground">
      <Icon aria-hidden className="size-3" />
      {copy.kinds[kind]}
    </span>
  );
}

type PersonLabelProps = {
  /** Whom it names; someone the user does not know of shows as someone. */
  person: Someone | undefined;
  className?: string;
};

/**
 * Someone on one line: their hue, their name and, for an agent or
 * integration, its badge and the person it acts for, "via Ada", marked once
 * it is deleted.
 */
export function PersonLabel({ person, className }: PersonLabelProps) {
  return (
    <span className={cn("inline-flex max-w-full min-w-0 items-center gap-1.5", className)}>
      <HueDot {...(person === undefined ? {} : { hue: person.hue })} className="size-1.5" />
      <span className="truncate">{person?.display_name ?? strings.databases.cells.someone}</span>
      {person !== undefined && actsForOwner(person) && (
        <>
          <KindBadge kind={person.kind} />
          {person.owner && (
            <span className="truncate text-muted-foreground">
              {copy.viaOwner(person.owner.display_name)}
            </span>
          )}
          {person.deleted === true && (
            <span className="shrink-0 text-muted-foreground">{copy.deletedMark}</span>
          )}
        </>
      )}
    </span>
  );
}
