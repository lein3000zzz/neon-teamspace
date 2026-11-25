import { cn } from "cn";
import type { PropertyOption } from "@/api/databases";
import { HueDot } from "@/components/nodes/hue-dot";
import { hueScope } from "@/components/nodes/hue";

type OptionChipProps = {
  option: Pick<PropertyOption, "name" | "hue">;
  className?: string;
};

/**
 * An option of a select, multi-select or status, as a pill in its own hue,
 * tinted and with a dot of it; an option without a hue is grey. The name
 * always says which it is, so the colour never has to.
 */
export function OptionChip({ option, className }: OptionChipProps) {
  const hue = option.hue ?? undefined;
  return (
    <span
      {...hueScope(hue)}
      className={cn(
        "inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-full border px-2 py-px text-xs leading-5",
        hue === undefined
          ? "border-border bg-muted text-muted-foreground"
          : "border-transparent bg-neon-soft text-foreground",
        className,
      )}
    >
      {hue !== undefined && <HueDot className="size-1.5" />}
      <span className="truncate">{option.name}</span>
    </span>
  );
}
