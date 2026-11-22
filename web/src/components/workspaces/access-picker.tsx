import { Lock, LockOpen, Users } from "lucide-react";
import type { TeamspaceAccess } from "@/api/teamspaces";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { strings } from "@/lib/strings";
import { NEW_TEAMSPACE_ACCESS } from "@/lib/workspace-rules";

type Choice = (typeof NEW_TEAMSPACE_ACCESS)[number];

const ICONS = { open: LockOpen, closed: Users, private: Lock } as const;

/** Open, closed or private, each said in a sentence: who sees the teamspace, and who joins it. */
export function AccessPicker({
  value,
  onChange,
  labelledBy,
}: {
  value: Choice;
  onChange: (access: Choice) => void;
  labelledBy: string;
}) {
  return (
    <ToggleGroup
      type="single"
      variant="cards"
      aria-labelledby={labelledBy}
      value={value}
      onValueChange={(next) => {
        const access = NEW_TEAMSPACE_ACCESS.find((option) => option === next);
        if (access !== undefined) {
          onChange(access);
        }
      }}
      className="grid w-full gap-2 sm:grid-cols-3"
    >
      {NEW_TEAMSPACE_ACCESS.map((access) => {
        const Icon = ICONS[access];
        return (
          <ToggleGroupItem
            key={access}
            value={access}
            aria-label={strings.access[access]}
            aria-describedby={`${labelledBy}-${access}`}
            className="h-full items-start justify-start p-3 text-left whitespace-normal"
          >
            <span className="flex items-center gap-2 font-medium">
              <Icon aria-hidden />
              {strings.access[access]}
            </span>
            <span id={`${labelledBy}-${access}`} className="text-xs text-muted-foreground">
              {strings.accessHints[access]}
            </span>
          </ToggleGroupItem>
        );
      })}
    </ToggleGroup>
  );
}

/** Narrows an access the server sent to one a teamspace can be changed to. */
export function choiceOf(access: TeamspaceAccess): Choice | undefined {
  return NEW_TEAMSPACE_ACCESS.find((option) => option === access);
}
